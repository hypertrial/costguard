use crate::{PrSummary, Project};
use costguard_dbt::{DbtModel, DbtProject};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

pub(crate) fn enrich_pr_summary(mut summary: PrSummary, project: &Project) -> PrSummary {
    let Some(dbt) = &project.dbt else {
        return summary;
    };
    let changed_set: HashSet<PathBuf> = summary.changed_files.iter().cloned().collect();
    let changed_model_ids = dbt
        .models
        .values()
        .filter(|model| {
            model
                .path
                .as_ref()
                .is_some_and(|path| changed_set.contains(path))
        })
        .map(|model| model.identity().to_string())
        .collect::<Vec<_>>();

    let graph = DbtDependencyGraph::from_project(dbt);
    summary.changed_models = graph.labels_for(&changed_model_ids);
    summary.changed_models.sort();

    let downstream_ids = graph.transitive_downstream(&changed_model_ids);
    summary.affected_downstream = graph.labels_for(&downstream_ids);
    summary.affected_downstream.sort();
    let affected_ids = changed_model_ids
        .iter()
        .chain(downstream_ids.iter())
        .cloned()
        .collect::<Vec<_>>();
    summary.affected_exposures = graph.affected_exposures(&affected_ids);
    summary.affected_exposures.sort();

    if !summary.changed_models.is_empty() {
        let selectors = summary
            .changed_models
            .iter()
            .map(|model| format!("{model}+"))
            .collect::<Vec<_>>()
            .join(" ");
        summary.recommended_dbt_command = Some(format!("dbt build --select {selectors}"));
    }
    summary
}

struct DbtDependencyGraph {
    reverse_dependencies: HashMap<String, Vec<String>>,
    exposure_dependencies: HashMap<String, Vec<String>>,
    labels: HashMap<String, String>,
}

impl DbtDependencyGraph {
    fn from_project(project: &DbtProject) -> Self {
        let name_counts = project
            .models
            .values()
            .fold(HashMap::new(), |mut counts, model| {
                *counts.entry(model.name.clone()).or_insert(0usize) += 1;
                counts
            });
        let labels = project
            .models
            .values()
            .map(|model| {
                (
                    model.identity().to_string(),
                    display_label(model, &name_counts),
                )
            })
            .collect::<HashMap<_, _>>();
        let name_to_ids = project.models.values().fold(
            HashMap::<String, Vec<String>>::new(),
            |mut map, model| {
                map.entry(model.name.clone())
                    .or_default()
                    .push(model.identity().to_string());
                if let Some(package) = &model.package_name {
                    map.entry(format!("{package}.{}", model.name))
                        .or_default()
                        .push(model.identity().to_string());
                }
                map
            },
        );
        let mut reverse_dependencies: HashMap<String, Vec<String>> = HashMap::new();

        for model in project.models.values() {
            let dependent = model.identity().to_string();
            for reference in &model.refs {
                let reference = normalize_dependency_id(reference, &labels, &name_to_ids);
                reverse_dependencies
                    .entry(reference)
                    .or_default()
                    .push(dependent.clone());
            }
            let graph_key = model.identity();
            if let Some(depends_on) = project.graph.depends_on.get(graph_key) {
                for dependency in depends_on {
                    let dependency_name =
                        normalize_dependency_id(dependency, &labels, &name_to_ids);
                    reverse_dependencies
                        .entry(dependency_name)
                        .or_default()
                        .push(dependent.clone());
                }
            }
        }

        for dependents in reverse_dependencies.values_mut() {
            dependents.sort();
            dependents.dedup();
        }

        let exposure_dependencies = project
            .exposures
            .values()
            .map(|exposure| {
                (
                    exposure.name.clone(),
                    exposure
                        .depends_on
                        .iter()
                        .map(|dependency| {
                            normalize_dependency_id(dependency, &labels, &name_to_ids)
                        })
                        .collect::<Vec<_>>(),
                )
            })
            .collect();

        Self {
            reverse_dependencies,
            exposure_dependencies,
            labels,
        }
    }

    fn transitive_downstream(&self, changed_model_ids: &[String]) -> Vec<String> {
        let changed = changed_model_ids.iter().cloned().collect::<HashSet<_>>();
        let mut seen = HashSet::new();
        let mut stack = changed_model_ids.to_vec();
        while let Some(model) = stack.pop() {
            if let Some(dependents) = self.reverse_dependencies.get(&model) {
                for dependent in dependents {
                    if changed.contains(dependent) || !seen.insert(dependent.clone()) {
                        continue;
                    }
                    stack.push(dependent.clone());
                }
            }
        }
        let mut downstream = seen.into_iter().collect::<Vec<_>>();
        downstream.sort();
        downstream
    }

    fn affected_exposures(&self, affected_model_ids: &[String]) -> Vec<String> {
        let affected = affected_model_ids.iter().cloned().collect::<HashSet<_>>();
        let mut exposures = self
            .exposure_dependencies
            .iter()
            .filter(|(_, dependencies)| {
                dependencies
                    .iter()
                    .any(|dependency| affected.contains(dependency))
            })
            .map(|(name, _)| name.clone())
            .collect::<Vec<_>>();
        exposures.sort();
        exposures
    }

    fn labels_for(&self, ids: &[String]) -> Vec<String> {
        ids.iter()
            .map(|id| self.labels.get(id).cloned().unwrap_or_else(|| id.clone()))
            .collect()
    }
}

fn display_label(model: &DbtModel, name_counts: &HashMap<String, usize>) -> String {
    if name_counts.get(&model.name).copied().unwrap_or(0) > 1 {
        if let Some(package) = &model.package_name {
            return format!("{package}.{}", model.name);
        }
    }
    model.name.clone()
}

fn normalize_dependency_id(
    dependency: &str,
    known_ids: &HashMap<String, String>,
    name_to_ids: &HashMap<String, Vec<String>>,
) -> String {
    if known_ids.contains_key(dependency) {
        return dependency.to_string();
    }
    let trimmed = dependency.trim();
    if let Some(inner) = trimmed
        .strip_prefix("ref(")
        .and_then(|value| value.strip_suffix(')'))
    {
        return normalize_dependency_id(inner.trim_matches(['"', '\'']), known_ids, name_to_ids);
    }
    let cleaned = trimmed.trim_matches(['"', '\'']).to_string();
    if let Some(ids) = name_to_ids.get(&cleaned) {
        if ids.len() == 1 {
            return ids[0].clone();
        }
    }
    if let Some(last) = cleaned.rsplit('.').next() {
        if let Some(ids) = name_to_ids.get(last) {
            if ids.len() == 1 {
                return ids[0].clone();
            }
        }
    }
    cleaned
}

#[cfg(test)]
mod tests {
    use super::*;
    use costguard_dbt::{DbtExposure, DbtModel};

    #[test]
    fn pr_summary_uses_transitive_manifest_graph_and_exposures() {
        let mut dbt = DbtProject::default();
        dbt.models.insert(
            "a".into(),
            DbtModel {
                node_id: Some("model.pkg.a".into()),
                name: "a".into(),
                path: Some(PathBuf::from("models/a.sql")),
                ..DbtModel::default()
            },
        );
        dbt.models.insert(
            "b".into(),
            DbtModel {
                node_id: Some("model.pkg.b".into()),
                name: "b".into(),
                path: Some(PathBuf::from("models/b.sql")),
                ..DbtModel::default()
            },
        );
        dbt.models.insert(
            "c".into(),
            DbtModel {
                node_id: Some("model.pkg.c".into()),
                name: "c".into(),
                path: Some(PathBuf::from("models/c.sql")),
                ..DbtModel::default()
            },
        );
        dbt.graph
            .depends_on
            .insert("model.pkg.b".into(), vec!["model.pkg.a".into()]);
        dbt.graph
            .depends_on
            .insert("model.pkg.c".into(), vec!["model.pkg.b".into()]);
        dbt.exposures.insert(
            "growth_dashboard".into(),
            DbtExposure {
                name: "growth_dashboard".into(),
                depends_on: vec!["model.pkg.c".into()],
            },
        );
        let project = Project {
            root: PathBuf::from("."),
            files: Vec::new(),
            dbt: Some(dbt),
        };
        let summary = enrich_pr_summary(
            PrSummary {
                changed_files: vec![PathBuf::from("models/a.sql")],
                ..PrSummary::default()
            },
            &project,
        );
        assert_eq!(summary.changed_models, vec!["a"]);
        assert_eq!(summary.affected_downstream, vec!["b", "c"]);
        assert_eq!(summary.affected_exposures, vec!["growth_dashboard"]);
    }

    #[test]
    fn pr_summary_falls_back_to_sql_refs_without_manifest() {
        let mut dbt = DbtProject::default();
        dbt.models.insert(
            "a".into(),
            DbtModel {
                name: "a".into(),
                path: Some(PathBuf::from("models/a.sql")),
                ..DbtModel::default()
            },
        );
        dbt.models.insert(
            "b".into(),
            DbtModel {
                name: "b".into(),
                path: Some(PathBuf::from("models/b.sql")),
                refs: vec!["a".into()],
                ..DbtModel::default()
            },
        );
        dbt.models.insert(
            "c".into(),
            DbtModel {
                name: "c".into(),
                path: Some(PathBuf::from("models/c.sql")),
                refs: vec!["b".into()],
                ..DbtModel::default()
            },
        );
        let project = Project {
            root: PathBuf::from("."),
            files: Vec::new(),
            dbt: Some(dbt),
        };
        let summary = enrich_pr_summary(
            PrSummary {
                changed_files: vec![PathBuf::from("models/a.sql")],
                ..PrSummary::default()
            },
            &project,
        );
        assert_eq!(summary.affected_downstream, vec!["b", "c"]);
    }

    #[test]
    fn pr_summary_uses_unique_ids_for_duplicate_model_names() {
        let mut dbt = DbtProject::default();
        for (package, path) in [
            ("alpha", "alpha/models/orders.sql"),
            ("beta", "beta/models/orders.sql"),
        ] {
            let id = format!("model.{package}.orders");
            dbt.models.insert(
                id.clone(),
                DbtModel {
                    unique_id: Some(id.clone()),
                    node_id: Some(id),
                    package_name: Some(package.into()),
                    name: "orders".into(),
                    path: Some(PathBuf::from(path)),
                    ..DbtModel::default()
                },
            );
        }
        dbt.models.insert(
            "model.beta.order_rollup".into(),
            DbtModel {
                unique_id: Some("model.beta.order_rollup".into()),
                node_id: Some("model.beta.order_rollup".into()),
                package_name: Some("beta".into()),
                name: "order_rollup".into(),
                path: Some(PathBuf::from("beta/models/order_rollup.sql")),
                ..DbtModel::default()
            },
        );
        dbt.graph.depends_on.insert(
            "model.beta.order_rollup".into(),
            vec!["model.beta.orders".into()],
        );
        let project = Project {
            root: PathBuf::from("."),
            files: Vec::new(),
            dbt: Some(dbt),
        };
        let summary = enrich_pr_summary(
            PrSummary {
                changed_files: vec![PathBuf::from("beta/models/orders.sql")],
                ..PrSummary::default()
            },
            &project,
        );
        assert_eq!(summary.changed_models, vec!["beta.orders"]);
        assert_eq!(summary.affected_downstream, vec!["order_rollup"]);
    }
}
