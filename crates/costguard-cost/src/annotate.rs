use crate::attribution::{
    attribute_findings, build_downstream_ids, build_exposure_counts, CostAttributionContext,
    ModelFeatureSummary,
};
use crate::catalog::{load_catalog, CatalogStats};
use crate::config::CostConfig;
use crate::model_cost::{
    build_model_cost_index, compute_realized_savings, summarize_project_costs, ModelCostEntry,
    ModelCostIndex, ProjectCostSummary,
};
use crate::observations::ObservationStats;
use crate::pricing::price_per_byte;
use crate::query_history::{load_query_history, QueryHistoryStats};
use crate::units::{price_bytes, BytesPerMonthEstimate};
use crate::Estimate;
use anyhow::Context;
use costguard_dbt::DbtProject;
use costguard_diagnostics::Diagnostic;
use costguard_project::{Framework, ModelMetadata, ProjectGraph};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default)]
pub struct CostInputs {
    pub catalog: Option<CatalogStats>,
    pub query_history: Option<QueryHistoryStats>,
    pub observations: Option<ObservationStats>,
    pub observations_before: Option<ObservationStats>,
    pub observations_after: Option<ObservationStats>,
}

impl CostInputs {
    pub fn load(root: &Path, config: &CostConfig) -> anyhow::Result<Self> {
        let catalog = load_optional_input(root, &config.inputs.catalog, load_catalog)?;
        let query_history =
            load_optional_input(root, &config.inputs.query_history, load_query_history)?;
        let observations = load_optional_observations(root, &config.inputs.observations, config)?;
        let observations_before =
            load_optional_observations(root, &config.inputs.observations_before, config)?;
        let observations_after =
            load_optional_observations(root, &config.inputs.observations_after, config)?;
        Ok(Self {
            catalog,
            query_history,
            observations,
            observations_before,
            observations_after,
        })
    }
}

fn load_optional_input<T, F>(
    root: &Path,
    path: &Option<PathBuf>,
    load: F,
) -> anyhow::Result<Option<T>>
where
    F: FnOnce(&Path) -> anyhow::Result<T>,
{
    if let Some(path) = path {
        let resolved = resolve_path(root, path);
        if resolved.exists() {
            Ok(Some(load(&resolved)?))
        } else {
            anyhow::bail!(
                "configured cost input file does not exist: {}",
                resolved.display()
            )
        }
    } else {
        Ok(None)
    }
}

fn load_optional_observations(
    root: &Path,
    path: &Option<PathBuf>,
    config: &CostConfig,
) -> anyhow::Result<Option<ObservationStats>> {
    if let Some(path) = path {
        let resolved = resolve_path(root, path);
        if resolved.exists() {
            let text = std::fs::read_to_string(&resolved)
                .with_context(|| format!("failed to read observations {}", resolved.display()))?;
            let bundle: costguard_protocol::CostObservationBundleV1 =
                serde_json::from_str(&text).context("invalid observations bundle JSON")?;
            crate::import::validate_cost_bundle(&bundle)?;
            Ok(Some(crate::observations::aggregate_bundle(&bundle, config)))
        } else {
            anyhow::bail!(
                "configured observations file does not exist: {}",
                resolved.display()
            )
        }
    } else {
        Ok(None)
    }
}

pub struct CostAnalysisResult {
    pub model_index: ModelCostIndex,
    pub summary: ProjectCostSummary,
}

fn resolve_path(root: &Path, path: &PathBuf) -> PathBuf {
    if path.is_absolute() {
        path.clone()
    } else {
        root.join(path)
    }
}

pub fn run_cost_analysis(
    config: &CostConfig,
    dbt: Option<&DbtProject>,
    inputs: &CostInputs,
    diagnostics: &mut [Diagnostic],
    features_by_path: &HashMap<PathBuf, ModelFeatureSummary>,
) -> CostAnalysisResult {
    run_cost_analysis_for_project(config, dbt, None, inputs, diagnostics, features_by_path)
}

pub fn run_cost_analysis_for_project(
    config: &CostConfig,
    dbt: Option<&DbtProject>,
    project: Option<&ProjectGraph>,
    inputs: &CostInputs,
    diagnostics: &mut [Diagnostic],
    features_by_path: &HashMap<PathBuf, ModelFeatureSummary>,
) -> CostAnalysisResult {
    if !config.enabled {
        return CostAnalysisResult {
            model_index: ModelCostIndex {
                models: HashMap::new(),
                by_path: HashMap::new(),
            },
            summary: ProjectCostSummary::default(),
        };
    }

    let mut model_index = build_model_cost_index(config, dbt, inputs);
    if let Some(project) = project {
        add_rocky_costs(config, project, inputs, &mut model_index);
    }
    let mut summary = summarize_project_costs(&model_index, config);

    let downstream_ids = project
        .map(project_downstream_ids)
        .unwrap_or_else(|| dbt.map(build_downstream_ids).unwrap_or_default());
    let downstream_counts = downstream_ids
        .iter()
        .map(|(id, downstream)| (id.clone(), downstream.len()))
        .collect();
    let exposure_counts = project
        .map(project_exposure_counts)
        .unwrap_or_else(|| dbt.map(build_exposure_counts).unwrap_or_default());
    let ctx = CostAttributionContext {
        config,
        model_index: &model_index,
        dbt,
        features_by_path,
        downstream_counts: &downstream_counts,
        downstream_ids: &downstream_ids,
        exposure_counts: &exposure_counts,
    };
    attribute_findings(diagnostics, &ctx, &mut summary);

    if let (Some(before), Some(after)) = (
        inputs.observations_before.as_ref(),
        inputs.observations_after.as_ref(),
    ) {
        summary.realized_savings = Some(compute_realized_savings(before, after, config));
    }

    CostAnalysisResult {
        model_index,
        summary,
    }
}

fn add_rocky_costs(
    config: &CostConfig,
    project: &ProjectGraph,
    inputs: &CostInputs,
    index: &mut ModelCostIndex,
) {
    let price = price_per_byte(config);
    for model in project
        .models
        .values()
        .filter(|model| model.framework == Framework::Rocky)
    {
        let keys = project_lookup_keys(model);
        let observed = inputs
            .observations
            .as_ref()
            .and_then(|stats| keys.iter().find_map(|key| stats.by_model.get(key)));
        let history = inputs
            .query_history
            .as_ref()
            .and_then(|stats| keys.iter().find_map(|key| stats.by_key.get(key)));
        let catalog_bytes = inputs.catalog.as_ref().and_then(|stats| {
            keys.iter()
                .find_map(|key| stats.bytes_by_key.get(key).copied())
        });
        if observed.is_none() && history.is_none() && catalog_bytes.is_none() {
            continue;
        }
        let runs_value = observed
            .map(|value| value.monthly_executions)
            .filter(|value| *value > 0.0)
            .or_else(|| history.map(|value| value.runs_per_month))
            .unwrap_or(30.0);
        let bytes_value = observed
            .and_then(|value| value.monthly_bytes)
            .map(|bytes| bytes / runs_value)
            .or_else(|| history.map(|value| value.bytes_per_run))
            .or_else(|| catalog_bytes.map(|value| value as f64))
            .unwrap_or(1.0)
            .max(1.0);
        let bytes = Estimate::from_point(bytes_value, Some(0.2));
        let runs = Estimate::from_point(runs_value.max(1.0), Some(0.15));
        let scan_volume = BytesPerMonthEstimate::from_bytes_and_runs(bytes, runs);
        let monthly_cost_usd = observed
            .and_then(|value| value.monthly_cost_usd)
            .or_else(|| price.map(|price| price_bytes(scan_volume, price).raw()));
        let (basis, grade) = if let Some(observed) = observed {
            (observed.basis.clone(), costguard_diagnostics::CostGrade::A)
        } else if history.is_some() {
            ("query-history".into(), costguard_diagnostics::CostGrade::B)
        } else {
            ("catalog".into(), costguard_diagnostics::CostGrade::B)
        };
        index.by_path.insert(model.path.clone(), model.id.clone());
        index.models.insert(
            model.id.clone(),
            ModelCostEntry {
                model_id: model.id.clone(),
                path: Some(model.path.clone()),
                bytes,
                runs_per_month: runs,
                scan_volume: scan_volume.raw(),
                monthly_cost_usd,
                gb_months: scan_volume.median() / 1_000_000_000.0,
                grade,
                estimation_basis: basis,
                observation_window: observed
                    .map(|value| (value.window_start.clone(), value.window_end.clone())),
                observation_age_days: observed.map(|value| value.age_days),
            },
        );
    }
}

fn project_lookup_keys(model: &ModelMetadata) -> Vec<String> {
    let mut keys = vec![model.id.clone(), model.name.clone()];
    if let Some(object) = &model.target.object {
        keys.push(object.clone());
        if let Some(schema) = &model.target.schema {
            keys.push(format!("{schema}.{object}"));
            if let Some(catalog) = &model.target.catalog {
                keys.push(format!("{catalog}.{schema}.{object}"));
            }
        }
    }
    keys.sort();
    keys.dedup();
    keys
}

fn project_downstream_ids(project: &ProjectGraph) -> HashMap<String, Vec<String>> {
    project
        .models
        .keys()
        .map(|id| {
            (
                id.clone(),
                project.transitive_downstream(std::slice::from_ref(id)),
            )
        })
        .collect()
}

fn project_exposure_counts(project: &ProjectGraph) -> HashMap<String, usize> {
    let mut counts = HashMap::new();
    for exposure in project.exposures.values() {
        for dependency in &exposure.depends_on {
            *counts.entry(dependency.clone()).or_insert(0) += 1;
        }
    }
    counts
}

pub fn total_p50_usd_per_month(diagnostics: &[Diagnostic]) -> f64 {
    diagnostics
        .iter()
        .filter_map(|d| {
            d.cost_estimate
                .as_ref()
                .and_then(|c| c.savings_p50_usd_per_month)
        })
        .sum()
}

pub fn total_savings_gb_months(diagnostics: &[Diagnostic]) -> f64 {
    diagnostics
        .iter()
        .filter_map(|d| d.cost_estimate.as_ref())
        .map(|c| c.relative_index)
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use costguard_diagnostics::{Confidence, Severity};
    use costguard_project::{Materialization, ModelMetadata, Target};

    #[test]
    fn annotates_relative_index() {
        let config = CostConfig {
            enabled: true,
            ..CostConfig::default()
        };
        let mut diagnostics = vec![Diagnostic {
            governance: Default::default(),
            rule_id: "SQLCOST014".into(),
            severity: Severity::Medium,
            path: PathBuf::from("models/marts/fct.sql"),
            line: 1,
            column: 1,
            span: None,
            message: "test".into(),
            risk: None,
            suggestion: None,
            confidence: Confidence::High,
            warehouse: None,
            source_provenance: None,
            compiled_line: None,
            compiled_column: None,
            cost_estimate: None,
            rule_precision_tier: None,
        }];
        let inputs = CostInputs::default();
        let summary =
            run_cost_analysis(&config, None, &inputs, &mut diagnostics, &HashMap::new()).summary;
        assert!(diagnostics[0].cost_estimate.is_some());
        assert!(
            diagnostics[0]
                .cost_estimate
                .as_ref()
                .unwrap()
                .relative_index
                > 0.0
        );
        assert!(summary.project_gb_months >= 0.0);
    }

    #[test]
    fn maps_rocky_models_by_neutral_id_without_dbt_priors() {
        let config = CostConfig {
            enabled: true,
            ..CostConfig::default()
        };
        let mut graph = ProjectGraph::default();
        graph.insert_model(ModelMetadata {
            id: "rocky.orders".into(),
            framework: Framework::Rocky,
            name: "orders".into(),
            path: PathBuf::from("models/orders.rocky"),
            materialization: Materialization::Incremental,
            strategy: Some("microbatch".into()),
            target: Target {
                catalog: Some("lake".into()),
                schema: Some("mart".into()),
                object: Some("orders".into()),
            },
            tags: Vec::new(),
            owners: Vec::new(),
            group: None,
            depends_on: Vec::new(),
        });
        graph.rebuild_indexes();
        let mut observations = ObservationStats::default();
        observations.by_model.insert(
            "rocky.orders".into(),
            crate::ObservedModelCost {
                monthly_cost_usd: Some(Estimate::from_point(100.0, Some(0.1))),
                monthly_credits: None,
                monthly_bytes: Some(1_000_000.0),
                monthly_executions: 10.0,
                window_start: "2026-01-01T00:00:00Z".into(),
                window_end: "2026-02-01T00:00:00Z".into(),
                age_days: 1.0,
                basis: "observed-usd".into(),
            },
        );
        let inputs = CostInputs {
            observations: Some(observations),
            ..CostInputs::default()
        };
        let result = run_cost_analysis_for_project(
            &config,
            None,
            Some(&graph),
            &inputs,
            &mut [],
            &HashMap::new(),
        );
        let entry = &result.model_index.models["rocky.orders"];
        assert_eq!(
            entry.path.as_deref(),
            Some(Path::new("models/orders.rocky"))
        );
        assert_eq!(entry.estimation_basis, "observed-usd");
        assert_eq!(result.summary.model_count, 1);
    }
}
