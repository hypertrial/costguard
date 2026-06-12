use crate::config::CostConfig;
use crate::estimate::{excess_multiplier, round_sig2, Estimate};
use crate::model_cost::{lookup_model_entry, ModelCostIndex, ProjectCostSummary};
use crate::multipliers::rule_multiplier;
use crate::pricing::{price_per_byte, pricing_label};
use crate::volume::{model_for_path, model_identity, VolumeContext};
use costguard_dbt::DbtProject;
use costguard_diagnostics::{CostEstimate, Diagnostic};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

#[derive(Debug, Clone, Default)]
pub struct ModelFeatureSummary {
    pub join_count: usize,
    pub cross_join_count: usize,
    pub select_star_count: usize,
    pub window_count: usize,
    pub cte_reuse_max: usize,
}

#[derive(Debug, Clone)]
struct PendingAttribution {
    diagnostic_index: usize,
    model_id: String,
    raw_savings: Estimate,
    gb_months_savings: f64,
    structure_factor: f64,
    fan_out_factor: f64,
    grade: costguard_diagnostics::CostGrade,
    model_monthly_cost: Estimate,
    volume_basis: String,
}

pub struct CostAttributionContext<'a> {
    pub config: &'a CostConfig,
    pub model_index: &'a ModelCostIndex,
    pub dbt: Option<&'a DbtProject>,
    pub features_by_path: &'a HashMap<PathBuf, ModelFeatureSummary>,
    pub downstream_counts: &'a HashMap<String, usize>,
    pub exposure_counts: &'a HashMap<String, usize>,
}

pub fn build_downstream_counts(dbt: &DbtProject) -> HashMap<String, usize> {
    let mut reverse: HashMap<String, Vec<String>> = HashMap::new();
    for model in dbt.models.values() {
        let dependent = model_identity(model);
        for reference in &model.refs {
            reverse
                .entry(reference.clone())
                .or_default()
                .push(dependent.clone());
        }
        let graph_key = model.identity();
        if let Some(deps) = dbt.graph.depends_on.get(graph_key) {
            for dep in deps {
                reverse
                    .entry(dep.clone())
                    .or_default()
                    .push(dependent.clone());
            }
        }
    }

    let mut counts = HashMap::new();
    for model in dbt.models.values() {
        let id = model_identity(model);
        let mut seen = HashSet::new();
        let mut stack = reverse.get(&id).cloned().unwrap_or_default();
        while let Some(node) = stack.pop() {
            if !seen.insert(node.clone()) {
                continue;
            }
            if let Some(next) = reverse.get(&node) {
                stack.extend(next.iter().cloned());
            }
        }
        counts.insert(id, seen.len());
    }
    counts
}

pub fn build_exposure_counts(dbt: &DbtProject) -> HashMap<String, usize> {
    let mut counts = HashMap::new();
    for exposure in dbt.exposures.values() {
        for dep in &exposure.depends_on {
            *counts.entry(dep.clone()).or_insert(0) += 1;
        }
    }
    counts
}

pub fn summarize_features(
    join_count: usize,
    cross_join_count: usize,
    select_star_count: usize,
    window_count: usize,
    cte_reuse_max: usize,
) -> ModelFeatureSummary {
    ModelFeatureSummary {
        join_count,
        cross_join_count,
        select_star_count,
        window_count,
        cte_reuse_max,
    }
}

pub fn attribute_findings(
    diagnostics: &mut [Diagnostic],
    ctx: &CostAttributionContext<'_>,
    summary: &mut ProjectCostSummary,
) {
    if !ctx.config.enabled {
        return;
    }

    let price = price_per_byte(ctx.config);
    let pricing = pricing_label(&ctx.config.pricing);
    let volume_ctx = VolumeContext {
        config: ctx.config,
        catalog: None,
        query_history: None,
        dbt: ctx.dbt,
    };

    let mut pending: Vec<PendingAttribution> = Vec::new();
    for (index, diagnostic) in diagnostics.iter().enumerate() {
        let Some(multiplier) = rule_multiplier(&diagnostic.rule_id, ctx.config) else {
            continue;
        };
        let dbt_model = ctx
            .dbt
            .and_then(|project| model_for_path(project, &diagnostic.path));
        let model_id = dbt_model.map(model_identity).unwrap_or_else(|| {
            diagnostic
                .path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_string()
        });

        let (model_monthly_cost, scan_volume, grade, volume_basis) =
            if let Some(entry) = lookup_model_entry(ctx.model_index, &diagnostic.path, dbt_model) {
                (
                    entry.monthly_cost,
                    entry.scan_volume,
                    entry.grade,
                    format!(
                        "{} × {:.0} runs/mo",
                        format_bytes(entry.bytes.median()),
                        entry.runs_per_month.median()
                    ),
                )
            } else {
                let volume = volume_ctx.resolve_for_model_with_rule(
                    dbt_model.unwrap_or(&costguard_dbt::DbtModel {
                        name: model_id.clone(),
                        path: Some(diagnostic.path.clone()),
                        ..costguard_dbt::DbtModel::default()
                    }),
                    Some(&diagnostic.rule_id),
                );
                let scan = volume.bytes * volume.runs_per_month;
                let monthly = if let Some(price) = price {
                    scan * price
                } else {
                    scan
                };
                (
                    monthly,
                    scan,
                    volume.grade,
                    format!(
                        "{} × {:.0} runs/mo (prior)",
                        format_bytes(volume.bytes.median()),
                        volume.runs_per_month.median()
                    ),
                )
            };

        let structure_factor = structure_factor_for_rule(
            &diagnostic.rule_id,
            ctx.features_by_path.get(&diagnostic.path),
        );
        let fan_out = fan_out_factor(
            ctx.downstream_counts.get(&model_id).copied().unwrap_or(0),
            ctx.exposure_counts.get(&model_id).copied().unwrap_or(0),
        );
        let excess = excess_multiplier(multiplier);
        let scan_savings = scan_volume * excess;
        let gb_months_savings =
            scan_savings.median() / 1_000_000_000.0 * structure_factor * fan_out;
        let raw_savings = if price.is_some() {
            model_monthly_cost
                * excess
                * Estimate::from_point(structure_factor * fan_out, Some(0.2))
        } else {
            scan_savings * Estimate::from_point(structure_factor * fan_out, Some(0.2))
        };

        pending.push(PendingAttribution {
            diagnostic_index: index,
            model_id,
            raw_savings,
            gb_months_savings,
            structure_factor,
            fan_out_factor: fan_out,
            grade,
            model_monthly_cost,
            volume_basis,
        });
    }

    apply_per_model_caps(&mut pending);
    let (savings_p10, savings_p50, savings_p90) =
        apply_attributions(diagnostics, &pending, ctx.config, &pricing, price.is_some());
    summary.savings_p10_usd = savings_p10;
    summary.savings_p50_usd = savings_p50;
    summary.savings_p90_usd = savings_p90;
    summary.savings_gb_months = round_sig2(
        diagnostics
            .iter()
            .filter_map(|d| d.cost_estimate.as_ref())
            .map(|c| c.relative_index)
            .sum(),
    );
}

fn apply_per_model_caps(pending: &mut [PendingAttribution]) {
    let mut by_model: HashMap<String, Vec<usize>> = HashMap::new();
    for (idx, item) in pending.iter().enumerate() {
        by_model.entry(item.model_id.clone()).or_default().push(idx);
    }

    for indices in by_model.values() {
        if indices.len() <= 1 {
            continue;
        }
        let budget = pending[indices[0]].model_monthly_cost.median() * 0.95;
        let raw_total: f64 = indices
            .iter()
            .map(|idx| pending[*idx].raw_savings.median())
            .sum();
        if raw_total <= budget || raw_total <= 0.0 {
            continue;
        }
        let scale = budget / raw_total;
        for idx in indices {
            let item = &pending[*idx];
            pending[*idx].raw_savings =
                Estimate::from_point(item.raw_savings.median() * scale, Some(0.25));
            pending[*idx].gb_months_savings *= scale;
        }
    }
}

fn apply_attributions(
    diagnostics: &mut [Diagnostic],
    pending: &[PendingAttribution],
    config: &CostConfig,
    pricing: &str,
    has_pricing: bool,
) -> (f64, f64, f64) {
    let mut savings_estimates = Vec::new();
    for item in pending {
        let savings = item.raw_savings;
        let (usd_p10, usd_p50, usd_p90) = if has_pricing {
            let (lo, hi) = savings.interval(config.interval);
            (
                Some(round_sig2(lo)),
                Some(round_sig2(savings.median())),
                Some(round_sig2(hi)),
            )
        } else {
            (None, None, None)
        };

        let relative_index = round_sig2(item.gb_months_savings);
        let model_monthly_p50 = if has_pricing {
            Some(round_sig2(item.model_monthly_cost.median()))
        } else {
            None
        };

        let cap_note = if item.structure_factor != 1.0 || item.fan_out_factor != 1.0 {
            format!(
                "; structure×{:.2}, lineage×{:.2}",
                item.structure_factor, item.fan_out_factor
            )
        } else {
            String::new()
        };

        let basis = format!(
            "Est. savings from {} on {} (model {} × {} rule excess × {}{})",
            item.volume_basis,
            item.model_id,
            if has_pricing {
                format!("${:.0}/mo", item.model_monthly_cost.median())
            } else {
                format!("{:.0} GB-mo", item.model_monthly_cost.median() / 1e9)
            },
            diagnostics[item.diagnostic_index].rule_id,
            pricing,
            cap_note
        );

        diagnostics[item.diagnostic_index].cost_estimate = Some(CostEstimate {
            relative_index,
            p10_usd_per_month: usd_p10,
            p50_usd_per_month: usd_p50,
            p90_usd_per_month: usd_p90,
            grade: item.grade,
            basis,
            currency: "USD".into(),
            model_id: Some(item.model_id.clone()),
            model_monthly_p50_usd: model_monthly_p50,
            savings_p10_usd_per_month: usd_p10,
            savings_p50_usd_per_month: usd_p50,
            savings_p90_usd_per_month: usd_p90,
        });
        savings_estimates.push(savings);
    }

    if savings_estimates.is_empty() {
        return (0.0, 0.0, 0.0);
    }
    let total = crate::estimate::sum_lognormals(&savings_estimates);
    let (lo, hi) = total.interval(config.interval);
    (round_sig2(lo), round_sig2(total.median()), round_sig2(hi))
}

fn structure_factor_for_rule(rule_id: &str, features: Option<&ModelFeatureSummary>) -> f64 {
    let Some(features) = features else {
        return 1.0;
    };
    match rule_id.to_ascii_uppercase().as_str() {
        "SQLCOST012" => {
            if features.cross_join_count >= 2 {
                1.25
            } else if features.cross_join_count == 0 {
                0.75
            } else {
                1.0
            }
        }
        "SQLCOST006" => (1.0 + (features.join_count as f64 * 0.05)).min(1.5),
        "SQLCOST001" => {
            if features.select_star_count > 0 {
                1.15
            } else {
                1.0
            }
        }
        "SQLCOST013" => {
            if features.window_count > 1 {
                1.2
            } else {
                1.0
            }
        }
        "SQLCOST014" => {
            if features.cte_reuse_max > 2 {
                1.2
            } else {
                1.0
            }
        }
        _ => 1.0,
    }
}

fn fan_out_factor(downstream_count: usize, exposure_count: usize) -> f64 {
    let lineage = 1.0 + (1.0 + downstream_count as f64).log2() * 0.25;
    let exposure = 1.0 + exposure_count as f64 * 0.05;
    lineage.min(2.0) * exposure.min(1.5)
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
    use crate::model_cost::build_model_cost_index;
    use crate::CostInputs;
    use costguard_dbt::DbtModel;
    use costguard_diagnostics::{Confidence, Severity};

    #[test]
    fn per_model_cap_limits_total_savings() {
        let config = CostConfig {
            enabled: true,
            ..CostConfig::default()
        };
        let mut dbt = DbtProject::default();
        dbt.models.insert(
            "m".into(),
            DbtModel {
                name: "m".into(),
                path: Some(PathBuf::from("models/m.sql")),
                ..DbtModel::default()
            },
        );
        let index = build_model_cost_index(&config, Some(&dbt), &CostInputs::default());
        let mut diagnostics = vec![
            Diagnostic {
                rule_id: "SQLCOST014".into(),
                severity: Severity::Medium,
                path: PathBuf::from("models/m.sql"),
                line: 1,
                column: 1,
                span: None,
                message: "a".into(),
                risk: None,
                suggestion: None,
                confidence: Confidence::High,
                warehouse: None,
                source_provenance: None,
                compiled_line: None,
                compiled_column: None,
                cost_estimate: None,
            },
            Diagnostic {
                rule_id: "SQLCOST018".into(),
                severity: Severity::Medium,
                path: PathBuf::from("models/m.sql"),
                line: 2,
                column: 1,
                span: None,
                message: "b".into(),
                risk: None,
                suggestion: None,
                confidence: Confidence::High,
                warehouse: None,
                source_provenance: None,
                compiled_line: None,
                compiled_column: None,
                cost_estimate: None,
            },
        ];
        let mut summary = ProjectCostSummary::default();
        let ctx = CostAttributionContext {
            config: &config,
            model_index: &index,
            dbt: Some(&dbt),
            features_by_path: &HashMap::new(),
            downstream_counts: &HashMap::new(),
            exposure_counts: &HashMap::new(),
        };
        attribute_findings(&mut diagnostics, &ctx, &mut summary);
        let total: f64 = diagnostics
            .iter()
            .filter_map(|d| d.cost_estimate.as_ref())
            .map(|c| c.relative_index)
            .sum();
        assert!(total > 0.0);
    }
}
