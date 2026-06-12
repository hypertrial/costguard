use crate::catalog::{load_catalog, CatalogStats};
use crate::config::CostConfig;
use crate::estimate::round_sig2;
use crate::multipliers::rule_multiplier;
use crate::pricing::{price_per_byte, pricing_label};
use crate::query_history::{load_query_history, QueryHistoryStats};
use crate::volume::{model_for_path, VolumeContext};
use costguard_dbt::DbtProject;
use costguard_diagnostics::{CostEstimate, Diagnostic};
use std::path::{Path, PathBuf};

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
                None
            }
        } else {
            None
        };
        let query_history = if let Some(path) = &config.inputs.query_history {
            let resolved = resolve_path(root, path);
            if resolved.exists() {
                Some(load_query_history(&resolved)?)
            } else {
                None
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

fn resolve_path(root: &Path, path: &PathBuf) -> PathBuf {
    if path.is_absolute() {
        path.clone()
    } else {
        root.join(path)
    }
}

pub fn annotate_diagnostics(
    diagnostics: &mut [Diagnostic],
    config: &CostConfig,
    dbt: Option<&DbtProject>,
    inputs: &CostInputs,
    root: &Path,
) {
    let _root = root;
    if !config.enabled {
        return;
    }

    let ctx = VolumeContext {
        config,
        catalog: inputs.catalog.as_ref(),
        query_history: inputs.query_history.as_ref(),
        dbt,
    };
    let price = price_per_byte(config);
    let pricing = pricing_label(&config.pricing);

    for diagnostic in diagnostics.iter_mut() {
        let Some(multiplier) = rule_multiplier(&diagnostic.rule_id, config) else {
            continue;
        };
        let model = dbt.and_then(|project| model_for_path(project, &diagnostic.path));
        let volume = if let Some(model) = model {
            ctx.resolve_for_model(model)
        } else {
            ctx.resolve_for_model(&costguard_dbt::DbtModel {
                name: diagnostic
                    .path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("unknown")
                    .to_string(),
                path: Some(diagnostic.path.clone()),
                ..costguard_dbt::DbtModel::default()
            })
        };

        let impact = volume.bytes * multiplier * volume.runs_per_month;
        let relative_index = round_sig2(impact.median() / 1_000_000_000.0);
        let (usd_p10, usd_p50, usd_p90) = if let Some(price) = price {
            let cost = impact * price;
            let (lo, hi) = cost.interval(config.interval);
            (
                Some(round_sig2(lo)),
                Some(round_sig2(cost.median())),
                Some(round_sig2(hi)),
            )
        } else {
            (None, None, None)
        };

        let basis = format!(
            "{} × {} rule impact × {:.0} runs/mo × {}",
            format_bytes(volume.bytes.median()),
            diagnostic.rule_id,
            volume.runs_per_month.median(),
            pricing
        );

        diagnostic.cost_estimate = Some(CostEstimate {
            relative_index,
            p10_usd_per_month: usd_p10,
            p50_usd_per_month: usd_p50,
            p90_usd_per_month: usd_p90,
            grade: volume.grade,
            basis,
            currency: "USD".into(),
        });
    }
}

pub fn total_p50_usd_per_month(diagnostics: &[Diagnostic]) -> f64 {
    diagnostics
        .iter()
        .filter_map(|d| d.cost_estimate.as_ref()?.p50_usd_per_month)
        .sum()
}

fn format_bytes(bytes: f64) -> String {
    if bytes >= 1_000_000_000_000.0 {
        format!("{:.1}TB", bytes / 1_000_000_000_000.0)
    } else if bytes >= 1_000_000_000.0 {
        format!("{:.0}GB", bytes / 1_000_000_000.0)
    } else {
        format!("{:.0}MB", bytes / 1_000_000.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use costguard_diagnostics::{Confidence, Severity};
    use std::path::PathBuf;

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
        let inputs = CostInputs {
            catalog: None,
            query_history: None,
        };
        annotate_diagnostics(&mut diagnostics, &config, None, &inputs, Path::new("."));
        assert!(diagnostics[0].cost_estimate.is_some());
        assert!(
            diagnostics[0]
                .cost_estimate
                .as_ref()
                .unwrap()
                .relative_index
                > 0.0
        );
    }
}
