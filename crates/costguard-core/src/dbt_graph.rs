use crate::owners::OwnerResolver;
use crate::{ChangedModelDetail, IndirectImpact, LoadedProject, PrSummary, RecommendedCommand};
use costguard_dbt::{DbtModel, DbtProject, DependencyGraph};
use costguard_project::Framework;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

pub(crate) fn enrich_pr_summary(
    summary: PrSummary,
    project: &LoadedProject,
    owners: &OwnerResolver,
) -> PrSummary {
    let summary = enrich_dbt_pr_summary(summary, project, owners);
    enrich_rocky_pr_summary(summary, project, owners)
}

fn enrich_dbt_pr_summary(
    mut summary: PrSummary,
    project: &LoadedProject,
    owners: &OwnerResolver,
) -> PrSummary {
    summary.changed_files.sort();
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
    let indirect = discover_indirect_impacts(dbt, &changed_set, &graph);
    summary.indirectly_affected = indirect.impacts;
    let mut affected_seed_ids = changed_model_ids.clone();
    affected_seed_ids.extend(indirect.model_ids);

    summary.changed_models = graph.labels_for(&changed_model_ids);
    summary.changed_models.sort();

    summary.changed_model_details = changed_model_ids
        .iter()
        .filter_map(|id| {
            let model = dbt.models.values().find(|model| model.identity() == id)?;
            let path = model.path.clone()?;
            let downstream_ids = graph.transitive_downstream(std::slice::from_ref(id));
            let affected_exposures = graph.affected_exposures(
                &[std::slice::from_ref(id), downstream_ids.as_slice()].concat(),
            );
            let affected_exposure_owners = affected_exposures
                .iter()
                .filter_map(|name| {
                    let exposure = dbt.exposures.get(name)?;
                    (!exposure.owners.is_empty()).then(|| (name.clone(), exposure.owners.clone()))
                })
                .collect();
            let mut tags = model.tags.clone();
            tags.sort();
            Some(ChangedModelDetail {
                id: id.clone(),
                name: graph
                    .label(id)
                    .map(str::to_string)
                    .unwrap_or_else(|| id.clone()),
                framework: Some("dbt".into()),
                path,
                tags,
                owners: owners.owners_for_model(model),
                group: model.group.clone(),
                affected_downstream: graph.labels_for(&downstream_ids),
                affected_exposures,
                affected_exposure_owners,
            })
        })
        .collect();
    summary
        .changed_model_details
        .sort_by(|left, right| left.path.cmp(&right.path));
    for detail in &summary.changed_model_details {
        for owner in &detail.owners {
            *summary.owner_summary.entry(owner.clone()).or_insert(0) += 1;
        }
    }

    let downstream_ids = graph.transitive_downstream(&affected_seed_ids);
    let mut affected_downstream = graph.labels_for(&downstream_ids);
    for impact in &summary.indirectly_affected {
        if !affected_downstream.contains(&impact.model) {
            affected_downstream.push(impact.model.clone());
        }
    }
    affected_downstream.sort();
    affected_downstream.dedup();
    summary.affected_downstream = affected_downstream;
    let affected_ids = affected_seed_ids
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
        summary.recommended_commands.push(RecommendedCommand {
            framework: "dbt".into(),
            command: summary.recommended_dbt_command.clone().unwrap_or_default(),
            scope: "changed_and_downstream".into(),
        });
    }
    summary
}

fn enrich_rocky_pr_summary(
    mut summary: PrSummary,
    project: &LoadedProject,
    owners: &OwnerResolver,
) -> PrSummary {
    let changed_set = summary
        .changed_files
        .iter()
        .cloned()
        .collect::<HashSet<_>>();
    let rocky_models = project
        .graph
        .models
        .values()
        .filter(|model| model.framework == Framework::Rocky)
        .collect::<Vec<_>>();
    if rocky_models.is_empty() {
        return summary;
    }
    let direct_paths = rocky_models
        .iter()
        .flat_map(|model| [model.path.clone(), model.path.with_extension("toml")])
        .collect::<HashSet<_>>();
    let global_change = changed_set
        .iter()
        .any(|path| project.rocky_inputs.contains(path) && !direct_paths.contains(path));
    let mut direct_ids = rocky_models
        .iter()
        .filter(|model| {
            changed_set.contains(&model.path)
                || changed_set.contains(&model.path.with_extension("toml"))
        })
        .map(|model| model.id.clone())
        .collect::<Vec<_>>();
    direct_ids.sort();

    for id in &direct_ids {
        let Some(model) = project.graph.model(id) else {
            continue;
        };
        let downstream = project
            .graph
            .transitive_downstream(std::slice::from_ref(id));
        let mut downstream_names = downstream
            .iter()
            .filter_map(|id| project.graph.model(id).map(|model| model.name.clone()))
            .collect::<Vec<_>>();
        downstream_names.sort();
        let model_owners = owners.owners_for_metadata(model);
        for owner in &model_owners {
            *summary.owner_summary.entry(owner.clone()).or_insert(0) += 1;
        }
        summary.changed_models.push(model.name.clone());
        summary.changed_model_details.push(ChangedModelDetail {
            id: model.id.clone(),
            name: model.name.clone(),
            framework: Some("rocky".into()),
            path: model.path.clone(),
            tags: model.tags.clone(),
            owners: model_owners,
            group: model.group.clone(),
            affected_downstream: downstream_names,
            affected_exposures: Vec::new(),
            affected_exposure_owners: Default::default(),
        });
        summary.recommended_commands.push(RecommendedCommand {
            framework: "rocky".into(),
            command: format!("rocky run --model {}", shell_word(&model.name)),
            scope: "direct_model".into(),
        });
    }

    let affected_seeds = if global_change {
        for model in &rocky_models {
            summary.indirectly_affected.push(IndirectImpact {
                model: model.name.clone(),
                reason: "global Rocky compile input changed".into(),
            });
        }
        rocky_models
            .iter()
            .map(|model| model.id.clone())
            .collect::<Vec<_>>()
    } else {
        direct_ids
    };
    let downstream = project.graph.transitive_downstream(&affected_seeds);
    summary.affected_downstream.extend(
        downstream
            .iter()
            .filter_map(|id| project.graph.model(id).map(|model| model.name.clone())),
    );
    summary.changed_models.sort();
    summary.changed_models.dedup();
    summary.affected_downstream.sort();
    summary.affected_downstream.dedup();
    summary
        .changed_model_details
        .sort_by(|left, right| left.path.cmp(&right.path));
    summary
}

fn shell_word(value: &str) -> String {
    if value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        value.into()
    } else {
        format!("'{}'", value.replace('\'', "'\"'\"'"))
    }
}

struct IndirectDiscovery {
    impacts: Vec<IndirectImpact>,
    model_ids: Vec<String>,
}

fn discover_indirect_impacts(
    dbt: &DbtProject,
    changed_files: &HashSet<PathBuf>,
    graph: &DbtDependencyGraph,
) -> IndirectDiscovery {
    let mut impacts = Vec::new();
    let mut model_ids = HashSet::new();

    if changed_files.iter().any(|path| is_dbt_project_file(path)) {
        impacts.push(IndirectImpact {
            model: "(all models)".into(),
            reason: "dbt_project.yml changed".into(),
        });
        for model in dbt.models.values() {
            model_ids.insert(model.identity().to_string());
        }
    }

    let changed_macros = graph.changed_macro_ids(changed_files);
    for macro_id in &changed_macros {
        let affected_macros = graph.transitive_macro_dependents(std::slice::from_ref(macro_id));
        for model_id in graph.models_for_macros(&affected_macros) {
            let label = graph
                .label(&model_id)
                .map(str::to_string)
                .unwrap_or(model_id.clone());
            if model_ids.insert(model_id.clone()) {
                let macro_name = graph.macro_label(macro_id);
                impacts.push(IndirectImpact {
                    model: label,
                    reason: format!("macro `{macro_name}` changed"),
                });
            }
        }
    }

    for path in changed_files {
        if is_source_yaml(path) {
            for model_id in graph.models_for_source_path(dbt, path) {
                let label = graph
                    .label(&model_id)
                    .map(str::to_string)
                    .unwrap_or(model_id.clone());
                if model_ids.insert(model_id.clone()) {
                    impacts.push(IndirectImpact {
                        model: label,
                        reason: format!("source metadata changed ({})", path.display()),
                    });
                }
            }
        }
    }

    impacts.sort_by(|left, right| {
        left.model
            .cmp(&right.model)
            .then(left.reason.cmp(&right.reason))
    });
    IndirectDiscovery {
        impacts,
        model_ids: model_ids.into_iter().collect(),
    }
}

fn is_dbt_project_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name == "dbt_project.yml")
}

fn is_source_yaml(path: &Path) -> bool {
    let normalized = costguard_diagnostics::posix_path(path).to_ascii_lowercase();
    normalized.contains("/sources/") || normalized.starts_with("sources/")
}

struct DbtDependencyGraph {
    graph: DependencyGraph,
    macro_path_to_id: HashMap<PathBuf, String>,
    macro_to_models: HashMap<String, Vec<String>>,
    macro_dependents: HashMap<String, Vec<String>>,
    source_dependencies: HashMap<String, Vec<String>>,
}

impl DbtDependencyGraph {
    fn from_project(project: &DbtProject) -> Self {
        let graph = DependencyGraph::from_project(project);
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
        let mut source_dependencies: HashMap<String, Vec<String>> = HashMap::new();
        for model in project.models.values() {
            let dependent = model.identity().to_string();
            if let Some(depends_on) = project.graph.depends_on.get(model.identity()) {
                for dependency in depends_on {
                    if dependency.starts_with("source.") {
                        let dependency_name =
                            normalize_dependency_id(dependency, &labels, &name_to_ids);
                        source_dependencies
                            .entry(dependency_name)
                            .or_default()
                            .push(dependent.clone());
                    }
                }
            }
        }
        for dependents in source_dependencies.values_mut() {
            dependents.sort();
            dependents.dedup();
        }

        let macro_path_to_id = project
            .macros
            .values()
            .filter_map(|dbt_macro| {
                dbt_macro
                    .path
                    .as_ref()
                    .map(|path| (path.clone(), dbt_macro.unique_id.clone()))
            })
            .collect::<HashMap<_, _>>();
        let mut macro_to_models: HashMap<String, Vec<String>> = HashMap::new();
        for (model_id, macros) in &project.graph.depends_on_macros {
            for macro_id in macros {
                macro_to_models
                    .entry(macro_id.clone())
                    .or_default()
                    .push(model_id.clone());
            }
        }
        for models in macro_to_models.values_mut() {
            models.sort();
            models.dedup();
        }
        let mut macro_dependents: HashMap<String, Vec<String>> = HashMap::new();
        for dbt_macro in project.macros.values() {
            for dependency in &dbt_macro.depends_on_macros {
                macro_dependents
                    .entry(dependency.clone())
                    .or_default()
                    .push(dbt_macro.unique_id.clone());
            }
        }
        for dependents in macro_dependents.values_mut() {
            dependents.sort();
            dependents.dedup();
        }

        Self {
            graph,
            macro_path_to_id,
            macro_to_models,
            macro_dependents,
            source_dependencies,
        }
    }

    fn changed_macro_ids(&self, changed_files: &HashSet<PathBuf>) -> Vec<String> {
        changed_files
            .iter()
            .filter_map(|path| self.macro_path_to_id.get(path).cloned())
            .collect()
    }

    fn transitive_macro_dependents(&self, changed_macro_ids: &[String]) -> HashSet<String> {
        let mut seen = changed_macro_ids.iter().cloned().collect::<HashSet<_>>();
        let mut stack = changed_macro_ids.to_vec();
        while let Some(macro_id) = stack.pop() {
            if let Some(dependents) = self.macro_dependents.get(&macro_id) {
                for dependent in dependents {
                    if seen.insert(dependent.clone()) {
                        stack.push(dependent.clone());
                    }
                }
            }
        }
        seen
    }

    fn models_for_macros(&self, macro_ids: &HashSet<String>) -> Vec<String> {
        let mut models = HashSet::new();
        for macro_id in macro_ids {
            if let Some(model_ids) = self.macro_to_models.get(macro_id) {
                models.extend(model_ids.iter().cloned());
            }
        }
        let mut models = models.into_iter().collect::<Vec<_>>();
        models.sort();
        models
    }

    fn models_for_source_path(&self, dbt: &DbtProject, path: &Path) -> Vec<String> {
        let source_name = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or_default()
            .to_string();
        let mut models = HashSet::new();
        for (source_id, dependents) in &self.source_dependencies {
            if source_id.contains(&format!(".{source_name}."))
                || source_id.ends_with(&format!(".{source_name}"))
            {
                models.extend(dependents.iter().cloned());
            }
        }
        if models.is_empty() {
            for model in dbt.models.values() {
                for source in &model.sources {
                    if source.source_name == source_name {
                        models.insert(model.identity().to_string());
                    }
                }
            }
        }
        let mut models = models.into_iter().collect::<Vec<_>>();
        models.sort();
        models
    }

    fn macro_label(&self, macro_id: &str) -> String {
        macro_id.rsplit('.').next().unwrap_or(macro_id).to_string()
    }

    fn label(&self, id: &str) -> Option<&str> {
        self.graph.label(id)
    }

    fn transitive_downstream(&self, changed_model_ids: &[String]) -> Vec<String> {
        self.graph.transitive_downstream(changed_model_ids, None)
    }

    fn affected_exposures(&self, affected_model_ids: &[String]) -> Vec<String> {
        self.graph.affected_exposures(affected_model_ids)
    }

    fn labels_for(&self, ids: &[String]) -> Vec<String> {
        self.graph.labels_for(ids)
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
    use costguard_dbt::{DbtExposure, DbtMacro, DbtModel};

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
                owners: Vec::new(),
            },
        );
        let project = LoadedProject {
            project: crate::Project {
                root: PathBuf::from("."),
                files: Vec::new(),
                dbt: Some(dbt),
            },
            graph: Default::default(),
            compiled_unmapped_paths: Default::default(),
            rocky_owned_paths: Default::default(),
            rocky_inputs: Default::default(),
        };
        let owners =
            OwnerResolver::load(PathBuf::from(".").as_path(), &Default::default()).unwrap();
        let summary = enrich_pr_summary(
            PrSummary {
                changed_files: vec![PathBuf::from("models/a.sql")],
                ..PrSummary::default()
            },
            &project,
            &owners,
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
        let project = LoadedProject {
            project: crate::Project {
                root: PathBuf::from("."),
                files: Vec::new(),
                dbt: Some(dbt),
            },
            graph: Default::default(),
            compiled_unmapped_paths: Default::default(),
            rocky_owned_paths: Default::default(),
            rocky_inputs: Default::default(),
        };
        let owners =
            OwnerResolver::load(PathBuf::from(".").as_path(), &Default::default()).unwrap();
        let summary = enrich_pr_summary(
            PrSummary {
                changed_files: vec![PathBuf::from("models/a.sql")],
                ..PrSummary::default()
            },
            &project,
            &owners,
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
        let project = LoadedProject {
            project: crate::Project {
                root: PathBuf::from("."),
                files: Vec::new(),
                dbt: Some(dbt),
            },
            graph: Default::default(),
            compiled_unmapped_paths: Default::default(),
            rocky_owned_paths: Default::default(),
            rocky_inputs: Default::default(),
        };
        let owners =
            OwnerResolver::load(PathBuf::from(".").as_path(), &Default::default()).unwrap();
        let summary = enrich_pr_summary(
            PrSummary {
                changed_files: vec![PathBuf::from("beta/models/orders.sql")],
                ..PrSummary::default()
            },
            &project,
            &owners,
        );
        assert_eq!(summary.changed_models, vec!["beta.orders"]);
        assert_eq!(summary.affected_downstream, vec!["order_rollup"]);
    }

    #[test]
    fn pr_summary_tracks_macro_driven_indirect_impact() {
        let mut dbt = DbtProject::default();
        dbt.macros.insert(
            "macro.pkg.helper".into(),
            DbtMacro {
                unique_id: "macro.pkg.helper".into(),
                name: "helper".into(),
                path: Some(PathBuf::from("macros/helper.sql")),
                depends_on_macros: Vec::new(),
            },
        );
        dbt.models.insert(
            "model.pkg.a".into(),
            DbtModel {
                unique_id: Some("model.pkg.a".into()),
                node_id: Some("model.pkg.a".into()),
                name: "a".into(),
                path: Some(PathBuf::from("models/a.sql")),
                ..DbtModel::default()
            },
        );
        dbt.models.insert(
            "model.pkg.b".into(),
            DbtModel {
                unique_id: Some("model.pkg.b".into()),
                node_id: Some("model.pkg.b".into()),
                name: "b".into(),
                path: Some(PathBuf::from("models/b.sql")),
                ..DbtModel::default()
            },
        );
        dbt.graph
            .depends_on_macros
            .insert("model.pkg.a".into(), vec!["macro.pkg.helper".into()]);
        dbt.graph
            .depends_on
            .insert("model.pkg.b".into(), vec!["model.pkg.a".into()]);
        let project = LoadedProject {
            project: crate::Project {
                root: PathBuf::from("."),
                files: Vec::new(),
                dbt: Some(dbt),
            },
            graph: Default::default(),
            compiled_unmapped_paths: Default::default(),
            rocky_owned_paths: Default::default(),
            rocky_inputs: Default::default(),
        };
        let owners =
            OwnerResolver::load(PathBuf::from(".").as_path(), &Default::default()).unwrap();
        let summary = enrich_pr_summary(
            PrSummary {
                changed_files: vec![PathBuf::from("macros/helper.sql")],
                ..PrSummary::default()
            },
            &project,
            &owners,
        );
        assert_eq!(summary.changed_models, Vec::<String>::new());
        assert_eq!(summary.indirectly_affected.len(), 1);
        assert_eq!(summary.indirectly_affected[0].model, "a");
        assert!(summary.indirectly_affected[0].reason.contains("helper"));
        assert_eq!(summary.affected_downstream, vec!["a", "b"]);
    }

    #[test]
    fn rocky_changes_include_descendants_commands_and_global_inputs() {
        let mut graph = costguard_project::ProjectGraph::default();
        for (id, name, path, depends_on) in [
            ("rocky.orders", "orders", "rocky/orders.rocky", vec![]),
            (
                "rocky.daily_orders",
                "daily_orders",
                "rocky/daily_orders.rocky",
                vec!["rocky.orders".to_string()],
            ),
        ] {
            graph.insert_model(costguard_project::ModelMetadata {
                id: id.into(),
                framework: costguard_project::Framework::Rocky,
                name: name.into(),
                path: path.into(),
                materialization: costguard_project::Materialization::View,
                strategy: Some("view".into()),
                target: Default::default(),
                tags: Vec::new(),
                owners: Vec::new(),
                group: None,
                depends_on,
            });
        }
        graph.rebuild_indexes();
        let owners =
            OwnerResolver::load(PathBuf::from(".").as_path(), &Default::default()).unwrap();
        let project = LoadedProject {
            project: crate::Project {
                root: PathBuf::from("."),
                files: Vec::new(),
                dbt: None,
            },
            graph,
            compiled_unmapped_paths: Default::default(),
            rocky_owned_paths: [
                PathBuf::from("rocky/orders.rocky"),
                PathBuf::from("rocky/daily_orders.rocky"),
            ]
            .into_iter()
            .collect(),
            rocky_inputs: [
                PathBuf::from("rocky.toml"),
                PathBuf::from("rocky/orders.rocky"),
                PathBuf::from("rocky/daily_orders.rocky"),
            ]
            .into_iter()
            .collect(),
        };

        let direct = enrich_pr_summary(
            PrSummary {
                changed_files: vec![PathBuf::from("rocky/orders.rocky")],
                ..PrSummary::default()
            },
            &project,
            &owners,
        );
        assert_eq!(direct.changed_models, vec!["orders"]);
        assert_eq!(direct.affected_downstream, vec!["daily_orders"]);
        assert_eq!(
            direct.changed_model_details[0].framework.as_deref(),
            Some("rocky")
        );
        assert_eq!(
            direct.recommended_commands[0].command,
            "rocky run --model orders"
        );

        let global = enrich_pr_summary(
            PrSummary {
                changed_files: vec![PathBuf::from("rocky.toml")],
                ..PrSummary::default()
            },
            &project,
            &owners,
        );
        assert_eq!(global.indirectly_affected.len(), 2);
        assert!(global
            .indirectly_affected
            .iter()
            .all(|impact| impact.reason == "global Rocky compile input changed"));
    }
}
