//! Shared dbt model dependency graph for lineage traversal.

use crate::{DbtModel, DbtProject};
use std::collections::{HashMap, HashSet};

pub struct DependencyGraph {
    reverse_dependencies: HashMap<String, Vec<String>>,
    exposure_dependencies: HashMap<String, Vec<String>>,
    labels: HashMap<String, String>,
}

impl DependencyGraph {
    pub fn from_project(project: &DbtProject) -> Self {
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

    pub fn reverse_dependencies(&self) -> &HashMap<String, Vec<String>> {
        &self.reverse_dependencies
    }

    pub fn transitive_downstream(&self, seed_ids: &[String], cap: Option<usize>) -> Vec<String> {
        let seeds = seed_ids.iter().cloned().collect::<HashSet<_>>();
        let mut seen = HashSet::new();
        let mut stack = if cap.is_some() {
            seed_ids
                .iter()
                .flat_map(|seed| {
                    self.reverse_dependencies
                        .get(seed)
                        .into_iter()
                        .flatten()
                        .cloned()
                })
                .collect::<Vec<_>>()
        } else {
            seed_ids.to_vec()
        };
        while let Some(model) = stack.pop() {
            if cap.is_some() {
                if !seen.insert(model.clone()) {
                    continue;
                }
                if seen.len() >= cap.unwrap_or(usize::MAX) {
                    break;
                }
                if let Some(dependents) = self.reverse_dependencies.get(&model) {
                    stack.extend(dependents.iter().cloned());
                }
            } else if let Some(dependents) = self.reverse_dependencies.get(&model) {
                for dependent in dependents {
                    if seeds.contains(dependent) || !seen.insert(dependent.clone()) {
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

    pub fn affected_exposures(&self, affected_model_ids: &[String]) -> Vec<String> {
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

    pub fn labels_for(&self, ids: &[String]) -> Vec<String> {
        ids.iter()
            .map(|id| self.labels.get(id).cloned().unwrap_or_else(|| id.clone()))
            .collect()
    }

    pub fn label(&self, id: &str) -> Option<&str> {
        self.labels.get(id).map(String::as_str)
    }
}

pub fn build_downstream_ids(dbt: &DbtProject, cap: Option<usize>) -> HashMap<String, Vec<String>> {
    let graph = DependencyGraph::from_project(dbt);
    dbt.models
        .values()
        .map(|model| {
            let id = model.identity().to_string();
            (
                id.clone(),
                graph.transitive_downstream(std::slice::from_ref(&id), cap),
            )
        })
        .collect()
}

pub fn build_downstream_counts(dbt: &DbtProject, cap: Option<usize>) -> HashMap<String, usize> {
    build_downstream_ids(dbt, cap)
        .into_iter()
        .map(|(id, ids)| (id, ids.len()))
        .collect()
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
    use crate::DbtModel;
    use std::path::PathBuf;

    #[test]
    fn duplicate_model_names_resolve_package_qualified_refs() {
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
        let graph = DependencyGraph::from_project(&dbt);
        let downstream = graph.transitive_downstream(&["model.beta.orders".into()], None);
        assert_eq!(downstream, vec!["model.beta.order_rollup".to_string()]);
        assert_eq!(
            graph.labels_for(&["model.beta.orders".into()]),
            vec!["beta.orders".to_string()]
        );
    }

    #[test]
    fn downstream_cap_is_deterministic_for_wide_graph() {
        let mut dbt = DbtProject::default();
        dbt.models.insert(
            "model.pkg.root".into(),
            DbtModel {
                unique_id: Some("model.pkg.root".into()),
                name: "root".into(),
                ..DbtModel::default()
            },
        );
        for index in 0..20 {
            let id = format!("model.pkg.child_{index:02}");
            dbt.graph
                .depends_on
                .insert(id.clone(), vec!["model.pkg.root".into()]);
            dbt.models.insert(
                id.clone(),
                DbtModel {
                    unique_id: Some(id),
                    name: format!("child_{index:02}"),
                    ..DbtModel::default()
                },
            );
        }
        let ids = build_downstream_ids(&dbt, Some(15))
            .remove("model.pkg.root")
            .expect("root downstream");
        assert_eq!(ids.len(), 15);
        assert_eq!(ids[0], "model.pkg.child_05");
        assert_eq!(ids[14], "model.pkg.child_19");
    }
}
