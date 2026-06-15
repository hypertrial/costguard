use crate::attribution::{
    attribute_findings, build_downstream_counts, build_exposure_counts, CostAttributionContext,
    ModelFeatureSummary,
};
use crate::catalog::{load_catalog, CatalogStats};
use crate::config::CostConfig;
use crate::model_cost::{
    build_model_cost_index, compute_realized_savings, summarize_project_costs, ModelCostIndex,
    ProjectCostSummary,
};
use crate::observations::ObservationStats;
use crate::query_history::{load_query_history, QueryHistoryStats};
use anyhow::Context;
use costguard_dbt::DbtProject;
use costguard_diagnostics::Diagnostic;
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
    if !config.enabled {
        return CostAnalysisResult {
            model_index: ModelCostIndex {
                models: HashMap::new(),
                by_path: HashMap::new(),
            },
            summary: ProjectCostSummary::default(),
        };
    }

    let model_index = build_model_cost_index(config, dbt, inputs);
    let mut summary = summarize_project_costs(&model_index, config);

    let downstream_counts = dbt.map(build_downstream_counts).unwrap_or_default();
    let exposure_counts = dbt.map(build_exposure_counts).unwrap_or_default();
    let ctx = CostAttributionContext {
        config,
        model_index: &model_index,
        dbt,
        features_by_path,
        downstream_counts: &downstream_counts,
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
}
