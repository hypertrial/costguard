//! Framework-neutral project metadata used by Costguard orchestration.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Framework {
    Dbt,
    Rocky,
}

impl Framework {
    pub fn label(self) -> &'static str {
        match self {
            Self::Dbt => "dbt",
            Self::Rocky => "rocky",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Materialization {
    Table,
    View,
    MaterializedView,
    Incremental,
    Ephemeral,
    DynamicTable,
    ContentAddressed,
    Other(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Target {
    pub catalog: Option<String>,
    pub schema: Option<String>,
    pub object: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelMetadata {
    pub id: String,
    pub framework: Framework,
    pub name: String,
    pub path: PathBuf,
    pub materialization: Materialization,
    pub strategy: Option<String>,
    pub target: Target,
    pub tags: Vec<String>,
    pub owners: Vec<String>,
    pub group: Option<String>,
    pub depends_on: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExposureMetadata {
    pub name: String,
    pub depends_on: Vec<String>,
    pub owners: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProjectGraph {
    pub models: BTreeMap<String, ModelMetadata>,
    pub exposures: BTreeMap<String, ExposureMetadata>,
    #[serde(skip)]
    path_to_id: BTreeMap<PathBuf, String>,
    #[serde(skip)]
    downstream: BTreeMap<String, Vec<String>>,
}

impl ProjectGraph {
    pub fn insert_model(&mut self, model: ModelMetadata) {
        self.path_to_id.insert(model.path.clone(), model.id.clone());
        self.models.insert(model.id.clone(), model);
    }

    pub fn rebuild_indexes(&mut self) {
        self.path_to_id.clear();
        self.downstream.clear();
        for model in self.models.values() {
            self.path_to_id.insert(model.path.clone(), model.id.clone());
            for dependency in &model.depends_on {
                self.downstream
                    .entry(dependency.clone())
                    .or_default()
                    .push(model.id.clone());
            }
        }
        for ids in self.downstream.values_mut() {
            ids.sort();
            ids.dedup();
        }
    }

    pub fn model_for_path(&self, path: &Path) -> Option<&ModelMetadata> {
        self.path_to_id.get(path).and_then(|id| self.models.get(id))
    }

    pub fn model(&self, id: &str) -> Option<&ModelMetadata> {
        self.models.get(id)
    }

    pub fn transitive_downstream(&self, seeds: &[String]) -> Vec<String> {
        let seed_set = seeds.iter().cloned().collect::<BTreeSet<_>>();
        let mut seen = seed_set.clone();
        let mut queue = VecDeque::from(seeds.to_vec());
        while let Some(id) = queue.pop_front() {
            for child in self.downstream.get(&id).into_iter().flatten() {
                if seen.insert(child.clone()) {
                    queue.push_back(child.clone());
                }
            }
        }
        seen.retain(|id| !seed_set.contains(id));
        seen.into_iter().collect()
    }

    pub fn affected_exposures(&self, model_ids: &[String]) -> Vec<String> {
        let affected = model_ids.iter().collect::<BTreeSet<_>>();
        self.exposures
            .values()
            .filter(|exposure| {
                exposure
                    .depends_on
                    .iter()
                    .any(|dependency| affected.contains(dependency))
            })
            .map(|exposure| exposure.name.clone())
            .collect()
    }

    pub fn merge(&mut self, other: ProjectGraph) -> Result<(), String> {
        for model in other.models.into_values() {
            if self.models.contains_key(&model.id) {
                return Err(format!("duplicate model id '{}'", model.id));
            }
            if let Some(existing) = self.model_for_path(&model.path) {
                return Err(format!(
                    "{} and {} both claim {}",
                    existing.id,
                    model.id,
                    model.path.display()
                ));
            }
            self.insert_model(model);
        }
        for exposure in other.exposures.into_values() {
            self.exposures.insert(exposure.name.clone(), exposure);
        }
        self.rebuild_indexes();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(id: &str, path: &str, depends_on: &[&str]) -> ModelMetadata {
        ModelMetadata {
            id: id.into(),
            framework: Framework::Rocky,
            name: id.into(),
            path: path.into(),
            materialization: Materialization::Table,
            strategy: None,
            target: Target::default(),
            tags: Vec::new(),
            owners: Vec::new(),
            group: None,
            depends_on: depends_on.iter().map(|value| (*value).into()).collect(),
        }
    }

    #[test]
    fn indexes_paths_and_transitive_downstream() {
        let mut graph = ProjectGraph::default();
        graph.insert_model(model("a", "models/a.sql", &[]));
        graph.insert_model(model("b", "models/b.sql", &["a"]));
        graph.insert_model(model("c", "models/c.sql", &["b"]));
        graph.rebuild_indexes();
        assert_eq!(
            graph.model_for_path(Path::new("models/b.sql")).unwrap().id,
            "b"
        );
        assert_eq!(graph.transitive_downstream(&["a".into()]), vec!["b", "c"]);
    }

    #[test]
    fn merge_keeps_framework_name_collisions_and_rejects_path_ownership_conflicts() {
        let mut dbt = ProjectGraph::default();
        let mut dbt_orders = model("model.pkg.orders", "dbt/orders.sql", &[]);
        dbt_orders.framework = Framework::Dbt;
        dbt_orders.name = "orders".into();
        dbt.insert_model(dbt_orders);
        dbt.rebuild_indexes();

        let mut rocky = ProjectGraph::default();
        let mut rocky_orders = model("rocky.orders", "rocky/orders.rocky", &[]);
        rocky_orders.name = "orders".into();
        rocky.insert_model(rocky_orders);
        rocky.rebuild_indexes();
        dbt.merge(rocky).unwrap();
        assert!(dbt.model("model.pkg.orders").is_some());
        assert!(dbt.model("rocky.orders").is_some());

        let mut conflict = ProjectGraph::default();
        conflict.insert_model(model("rocky.conflict", "dbt/orders.sql", &[]));
        conflict.rebuild_indexes();
        assert!(dbt
            .merge(conflict)
            .unwrap_err()
            .contains("both claim dbt/orders.sql"));
    }
}
