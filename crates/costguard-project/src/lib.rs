//! Framework-neutral project metadata used by Costguard orchestration.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExposureMetadata {
    pub name: String,
    pub depends_on: Vec<String>,
    pub owners: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectGraphError {
    DuplicateModelId(String),
    DuplicateModelPath {
        existing_id: String,
        incoming_id: String,
        path: PathBuf,
    },
    DuplicateExposure(String),
}

impl fmt::Display for ProjectGraphError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateModelId(id) => write!(formatter, "duplicate model id '{id}'"),
            Self::DuplicateModelPath {
                existing_id,
                incoming_id,
                path,
            } => write!(
                formatter,
                "{existing_id} and {incoming_id} both claim {}",
                path.display()
            ),
            Self::DuplicateExposure(name) => {
                write!(formatter, "duplicate exposure name '{name}'")
            }
        }
    }
}

impl std::error::Error for ProjectGraphError {}

#[derive(Debug, Clone, Default)]
pub struct ProjectGraph {
    models: BTreeMap<String, ModelMetadata>,
    exposures: BTreeMap<String, ExposureMetadata>,
    path_to_id: BTreeMap<PathBuf, String>,
    downstream: BTreeMap<String, Vec<String>>,
}

impl ProjectGraph {
    pub fn from_parts(
        models: impl IntoIterator<Item = ModelMetadata>,
        exposures: impl IntoIterator<Item = ExposureMetadata>,
    ) -> Result<Self, ProjectGraphError> {
        let mut graph = Self::default();
        for model in models {
            graph.insert_model(model)?;
        }
        for exposure in exposures {
            graph.insert_exposure(exposure)?;
        }
        Ok(graph)
    }

    pub fn models(&self) -> &BTreeMap<String, ModelMetadata> {
        &self.models
    }

    pub fn exposures(&self) -> &BTreeMap<String, ExposureMetadata> {
        &self.exposures
    }

    pub fn insert_model(&mut self, model: ModelMetadata) -> Result<(), ProjectGraphError> {
        if self.models.contains_key(&model.id) {
            return Err(ProjectGraphError::DuplicateModelId(model.id));
        }
        if let Some(existing_id) = self.path_to_id.get(&model.path) {
            return Err(ProjectGraphError::DuplicateModelPath {
                existing_id: existing_id.clone(),
                incoming_id: model.id,
                path: model.path,
            });
        }
        for dependency in &model.depends_on {
            let children = self.downstream.entry(dependency.clone()).or_default();
            if !children.contains(&model.id) {
                children.push(model.id.clone());
                children.sort();
            }
        }
        self.path_to_id.insert(model.path.clone(), model.id.clone());
        self.models.insert(model.id.clone(), model);
        Ok(())
    }

    pub fn insert_exposure(&mut self, exposure: ExposureMetadata) -> Result<(), ProjectGraphError> {
        if self.exposures.contains_key(&exposure.name) {
            return Err(ProjectGraphError::DuplicateExposure(exposure.name));
        }
        self.exposures.insert(exposure.name.clone(), exposure);
        Ok(())
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

    pub fn merge(&mut self, other: ProjectGraph) -> Result<(), ProjectGraphError> {
        let mut merged = self.clone();
        for model in other.models.into_values() {
            merged.insert_model(model)?;
        }
        for exposure in other.exposures.into_values() {
            merged.insert_exposure(exposure)?;
        }
        *self = merged;
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
        graph.insert_model(model("a", "models/a.sql", &[])).unwrap();
        graph
            .insert_model(model("b", "models/b.sql", &["a"]))
            .unwrap();
        graph
            .insert_model(model("c", "models/c.sql", &["b"]))
            .unwrap();
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
        dbt.insert_model(dbt_orders).unwrap();

        let mut rocky = ProjectGraph::default();
        let mut rocky_orders = model("rocky.orders", "rocky/orders.rocky", &[]);
        rocky_orders.name = "orders".into();
        rocky.insert_model(rocky_orders).unwrap();
        dbt.merge(rocky).unwrap();
        assert!(dbt.model("model.pkg.orders").is_some());
        assert!(dbt.model("rocky.orders").is_some());

        let mut conflict = ProjectGraph::default();
        conflict
            .insert_model(model("rocky.conflict", "dbt/orders.sql", &[]))
            .unwrap();
        assert!(dbt
            .merge(conflict)
            .unwrap_err()
            .to_string()
            .contains("both claim dbt/orders.sql"));
    }

    #[test]
    fn duplicate_insertions_do_not_mutate_the_graph() {
        let mut graph = ProjectGraph::default();
        graph.insert_model(model("a", "models/a.sql", &[])).unwrap();
        let before = graph.clone();
        assert!(matches!(
            graph.insert_model(model("a", "models/other.sql", &[])),
            Err(ProjectGraphError::DuplicateModelId(_))
        ));
        assert_eq!(graph.models(), before.models());
        assert_eq!(
            graph.model_for_path(Path::new("models/a.sql")).unwrap().id,
            "a"
        );

        assert!(matches!(
            graph.insert_model(model("b", "models/a.sql", &[])),
            Err(ProjectGraphError::DuplicateModelPath { .. })
        ));
        assert_eq!(graph.models(), before.models());
    }

    #[test]
    fn merge_is_transactional_and_rejects_duplicate_exposures() {
        let exposure = |name: &str| ExposureMetadata {
            name: name.into(),
            depends_on: Vec::new(),
            owners: Vec::new(),
        };
        let mut graph =
            ProjectGraph::from_parts([model("a", "models/a.sql", &[])], [exposure("dashboard")])
                .unwrap();
        let path_conflict = ProjectGraph::from_parts(
            [
                model("b", "models/b.sql", &["a"]),
                model("c", "models/a.sql", &[]),
            ],
            Vec::<ExposureMetadata>::new(),
        )
        .unwrap();
        assert!(matches!(
            graph.merge(path_conflict),
            Err(ProjectGraphError::DuplicateModelPath { .. })
        ));
        assert!(graph.model("b").is_none());

        let conflict = ProjectGraph::from_parts(
            [model("b", "models/b.sql", &["a"])],
            [exposure("dashboard")],
        )
        .unwrap();
        assert!(matches!(
            graph.merge(conflict),
            Err(ProjectGraphError::DuplicateExposure(_))
        ));
        assert!(graph.model("b").is_none());
    }
}
