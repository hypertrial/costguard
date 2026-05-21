use crate::{PrSummary, Project};
use costguard_dbt::DbtProject;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

pub(crate) fn enrich_pr_summary(mut summary: PrSummary, project: &Project) -> PrSummary {
    let Some(dbt) = &project.dbt else {
        return summary;
    };
    let changed_set: HashSet<PathBuf> = summary.changed_files.iter().cloned().collect();
    summary.changed_models = dbt
        .models
        .values()
        .filter(|model| {
            model
                .path
                .as_ref()
                .is_some_and(|path| changed_set.contains(path))
        })
        .map(|model| model.name.clone())
        .collect();
    summary.changed_models.sort();

    let graph = DbtDependencyGraph::from_project(dbt);
    summary.affected_downstream = graph.transitive_downstream(&summary.changed_models);
    summary.affected_downstream.sort();
    summary.affected_exposures = graph.affected_exposures(
        &summary
            .changed_models
            .iter()
            .chain(summary.affected_downstream.iter())
            .cloned()
            .collect::<Vec<_>>(),
    );
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
}

impl DbtDependencyGraph {
    fn from_project(project: &DbtProject) -> Self {
        let node_to_model_name = project
            .models
            .values()
            .filter_map(|model| {
                model
                    .node_id
                    .as_ref()
                    .map(|node_id| (node_id.clone(), model.name.clone()))
            })
            .collect::<HashMap<_, _>>();
        let mut reverse_dependencies: HashMap<String, Vec<String>> = HashMap::new();

        for model in project.models.values() {
            let dependent = model.name.clone();
            for reference in &model.refs {
                reverse_dependencies
                    .entry(reference.clone())
                    .or_default()
                    .push(dependent.clone());
            }
            let graph_key = model.node_id.as_ref().unwrap_or(&model.name);
            if let Some(depends_on) = project.graph.depends_on.get(graph_key) {
                for dependency in depends_on {
                    let dependency_name =
                        normalize_dependency_name(dependency, &node_to_model_name);
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
                            normalize_dependency_name(dependency, &node_to_model_name)
                        })
                        .collect::<Vec<_>>(),
                )
            })
            .collect();

        Self {
            reverse_dependencies,
            exposure_dependencies,
        }
    }

    fn transitive_downstream(&self, changed_models: &[String]) -> Vec<String> {
        let changed = changed_models.iter().cloned().collect::<HashSet<_>>();
        let mut seen = HashSet::new();
        let mut stack = changed_models.to_vec();
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

    fn affected_exposures(&self, affected_models: &[String]) -> Vec<String> {
        let affected = affected_models.iter().cloned().collect::<HashSet<_>>();
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
}

fn normalize_dependency_name(
    dependency: &str,
    node_to_model_name: &HashMap<String, String>,
) -> String {
    if let Some(name) = node_to_model_name.get(dependency) {
        return name.clone();
    }
    let trimmed = dependency.trim();
    if let Some(inner) = trimmed
        .strip_prefix("ref(")
        .and_then(|value| value.strip_suffix(')'))
    {
        return inner.trim_matches(['"', '\'']).to_string();
    }
    trimmed
        .rsplit('.')
        .next()
        .unwrap_or(trimmed)
        .trim_matches(['"', '\''])
        .to_string()
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
}
