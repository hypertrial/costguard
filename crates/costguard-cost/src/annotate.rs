use crate::attribution::{
    attribute_findings, build_downstream_counts, build_exposure_counts, CostAttributionContext,
    ModelFeatureSummary,
};
use crate::catalog::{load_catalog, CatalogStats};
use crate::config::CostConfig;
use crate::model_cost::{
    build_model_cost_index, summarize_project_costs, ModelCostIndex, ProjectCostSummary,
};
use crate::query_history::{load_query_history, QueryHistoryStats};
use costguard_dbt::DbtProject;
use costguard_diagnostics::Diagnostic;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default)]
pub struct CostInputs {
    pub catalog: Option<CatalogStats>,
    pub query_history: Option<QueryHistoryStats>,
}

impl CostInputs {
    pub fn load(root: &Path, config: &CostConfig) -> anyhow::Result<Self> {
        let catalog = if let Some(path) = &config.inputs.catalog {
            let resolved = resolve_path(root, path);
            if resolved.exists() {
                Some(load_catalog(&resolved)?)
            } else {
                anyhow::bail!(
                    "configured catalog file does not exist: {}",
                    resolved.display()
                )
            }
        } else {
            None
        };
        let query_history = if let Some(path) = &config.inputs.query_history {
            let resolved = resolve_path(root, path);
            if resolved.exists() {
                Some(load_query_history(&resolved)?)
            } else {
                anyhow::bail!(
                    "configured query history file does not exist: {}",
                    resolved.display()
                )
            }
        } else {
            None
        };
        Ok(Self {
            catalog,
            query_history,
        })
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
    CostAnalysisResult {
        model_index,
        summary,
    }
}

/// Backward-compatible entry point used by scan pipeline.
pub fn annotate_diagnostics(
    diagnostics: &mut [Diagnostic],
    config: &CostConfig,
    dbt: Option<&DbtProject>,
    inputs: &CostInputs,
    _root: &Path,
    features_by_path: &HashMap<PathBuf, ModelFeatureSummary>,
) -> ProjectCostSummary {
    run_cost_analysis(config, dbt, inputs, diagnostics, features_by_path).summary
}

pub fn total_p50_usd_per_month(diagnostics: &[Diagnostic]) -> f64 {
    diagnostics
        .iter()
        .filter_map(|d| {
            d.cost_estimate
                .as_ref()
                .and_then(|c| c.savings_p50_usd_per_month.or(c.p50_usd_per_month))
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
        let summary = annotate_diagnostics(
            &mut diagnostics,
            &config,
            None,
            &inputs,
            Path::new("."),
            &HashMap::new(),
        );
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
