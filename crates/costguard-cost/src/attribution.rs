use crate::config::CostConfig;
use crate::estimate::{
    combined_multiplier, round_sig2, savings_fraction, sum_lognormals, Estimate,
};
use crate::model_cost::{lookup_model_entry, CostFigure, ModelCostIndex, ProjectCostSummary};
use crate::multipliers::{is_cost_bearing_rule, rule_multiplier, unestimated_reason};
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
    rule_multiplier: Estimate,
    savings_fraction: Estimate,
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
    // ponytail: stop at 15 descendants — fan_out_factor() saturates there; recompute if formula changes
    const MAX_DOWNSTREAM: usize = 15;
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
            if seen.len() >= MAX_DOWNSTREAM {
                break;
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
    let has_pricing = price.is_some();
    let volume_ctx = VolumeContext {
        config: ctx.config,
        observations: None,
        catalog: None,
        query_history: None,
        dbt: ctx.dbt,
    };

    let mut pending: Vec<PendingAttribution> = Vec::new();
    let mut rules_estimated = 0usize;
    let mut rules_unestimated = 0usize;

    // ponytail: index loop so we can assign cost_estimate while reading other fields
    #[allow(clippy::needless_range_loop)]
    for index in 0..diagnostics.len() {
        let rule_id = diagnostics[index].rule_id.clone();
        if !is_cost_bearing_rule(&rule_id) {
            continue;
        }
        let Some(multiplier) = rule_multiplier(&rule_id, ctx.config) else {
            if let Some(reason) = unestimated_reason(&rule_id, ctx.config) {
                rules_unestimated += 1;
                diagnostics[index].cost_estimate = Some(CostEstimate {
                    relative_index: 0.0,
                    grade: costguard_diagnostics::CostGrade::C,
                    basis: format!("Unestimated: {reason}"),
                    currency: "USD".into(),
                    model_id: None,
                    model_monthly_p50_usd: None,
                    savings_p10_usd_per_month: None,
                    savings_p50_usd_per_month: None,
                    savings_p90_usd_per_month: None,
                    current_cost_p50_usd_per_month: None,
                    post_fix_cost_p50_usd_per_month: None,
                    unestimated_reason: Some(reason),
                });
            }
            continue;
        };
        rules_estimated += 1;

        let diagnostic = &diagnostics[index];
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
                        "{} × {:.0} runs/mo ({})",
                        format_bytes(entry.bytes.median()),
                        entry.runs_per_month.median(),
                        entry.estimation_basis
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
                let monthly = volume.observed_monthly_cost.unwrap_or_else(|| {
                    if let Some(price) = price {
                        scan * price
                    } else {
                        scan
                    }
                });
                (
                    monthly,
                    scan,
                    volume.grade,
                    format!(
                        "{} × {:.0} runs/mo ({})",
                        format_bytes(volume.bytes.median()),
                        volume.runs_per_month.median(),
                        volume.estimation_basis
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
        let fraction = savings_fraction(multiplier);
        let attribution_weight = Estimate::from_point(structure_factor * fan_out, Some(0.2));
        let scan_savings = scan_volume * fraction * attribution_weight;
        let gb_months_savings = scan_savings.median() / 1_000_000_000.0;
        let raw_savings = if has_pricing {
            model_monthly_cost * fraction * attribution_weight
        } else {
            scan_savings
        };

        pending.push(PendingAttribution {
            diagnostic_index: index,
            model_id,
            rule_multiplier: multiplier,
            savings_fraction: fraction,
            raw_savings,
            gb_months_savings,
            structure_factor,
            fan_out_factor: fan_out,
            grade,
            model_monthly_cost,
            volume_basis,
        });
    }

    let per_model = apply_per_model_caps(&mut pending);
    let (savings_p10, savings_p50, savings_p90) =
        apply_attributions(diagnostics, &pending, ctx.config, &pricing, has_pricing);

    // ponytail: project_current matches summarize_project_costs (all indexed models);
    // per_model caps cover only models with findings — savings outside the index are minor.
    let project_current = sum_lognormals(
        &ctx.model_index
            .models
            .values()
            .map(|entry| entry.monthly_cost)
            .collect::<Vec<_>>(),
    );
    let (potential_total, post_fix_total) = if per_model.is_empty() {
        (Estimate::from_point(0.001, Some(0.5)), project_current)
    } else {
        let model_savings: Vec<Estimate> = per_model
            .iter()
            .map(|(c, p)| Estimate::from_point((c.median() - p.median()).max(0.001), Some(0.15)))
            .collect();
        let potential_total = sum_lognormals(&model_savings);
        let post_fix_median = (project_current.median() - potential_total.median()).max(0.001);
        let post_fix_total = Estimate::from_point(post_fix_median, Some(0.15));
        (potential_total, post_fix_total)
    };

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
    summary.post_fix_cost = CostFigure::from_estimate(
        post_fix_total,
        ctx.config,
        "post-fix-counterfactual",
        has_pricing,
    );
    summary.potential_savings = CostFigure::from_estimate(
        potential_total,
        ctx.config,
        "potential-savings",
        has_pricing,
    );
    summary.coverage.rules_estimated = rules_estimated;
    summary.coverage.rules_unestimated = rules_unestimated;
}

fn apply_per_model_caps(pending: &mut [PendingAttribution]) -> Vec<(Estimate, Estimate)> {
    let mut by_model: HashMap<String, Vec<usize>> = HashMap::new();
    for (idx, item) in pending.iter().enumerate() {
        by_model.entry(item.model_id.clone()).or_default().push(idx);
    }

    let mut per_model = Vec::new();
    for indices in by_model.values() {
        if indices.is_empty() {
            continue;
        }
        let current = pending[indices[0]].model_monthly_cost;
        let multipliers: Vec<Estimate> = indices
            .iter()
            .map(|idx| pending[*idx].rule_multiplier)
            .collect();
        let combined = combined_multiplier(&multipliers);
        let max_combined = combined.median().max(1.001);
        let capped_combined = max_combined.min(20.0);
        let post_fix = current / Estimate::from_point(capped_combined, Some(0.15));
        let savings_median = (current.median() - post_fix.median()).max(0.001);
        let total_savings = Estimate::from_point(savings_median, Some(0.2));
        per_model.push((current, post_fix));

        let weights: Vec<f64> = indices
            .iter()
            .map(|idx| pending[*idx].savings_fraction.median())
            .collect();
        let weight_sum: f64 = weights.iter().sum();
        if weight_sum <= 0.0 {
            continue;
        }

        for (idx, weight) in indices.iter().zip(weights) {
            let share = weight / weight_sum;
            let gb_before = pending[*idx].gb_months_savings;
            let fraction_median = pending[*idx].savings_fraction.median().max(0.001);
            pending[*idx].raw_savings = total_savings * Estimate::from_point(share, Some(0.2));
            pending[*idx].gb_months_savings = gb_before * (share / fraction_median);
        }
    }
    per_model
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
        let combined = combined_multiplier(&[item.rule_multiplier]);
        let post_fix = item.model_monthly_cost / combined;

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
        let post_fix_p50 = if has_pricing {
            Some(round_sig2(post_fix.median()))
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
            "Est. savings from {} on {} (model {} × {} rule fraction {}{})",
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
            grade: item.grade,
            basis,
            currency: "USD".into(),
            model_id: Some(item.model_id.clone()),
            model_monthly_p50_usd: model_monthly_p50,
            savings_p10_usd_per_month: usd_p10,
            savings_p50_usd_per_month: usd_p50,
            savings_p90_usd_per_month: usd_p90,
            current_cost_p50_usd_per_month: model_monthly_p50,
            post_fix_cost_p50_usd_per_month: post_fix_p50,
            unestimated_reason: None,
        });
        savings_estimates.push(savings);
    }

    if savings_estimates.is_empty() {
        return (0.0, 0.0, 0.0);
    }
    let total = sum_lognormals(&savings_estimates);
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
    use crate::model_cost::{build_model_cost_index, summarize_project_costs};
    use crate::CostInputs;
    use costguard_dbt::DbtModel;
    use costguard_diagnostics::{Confidence, Severity};

    #[test]
    fn per_model_cap_limits_total_savings() {
        let mut config = CostConfig {
            enabled: true,
            ..CostConfig::default()
        };
        config.pricing = crate::config::CostPricingSection {
            model: Some("scan".into()),
            usd_per_tb: Some(6.25),
            usd_per_credit: None,
            tb_per_credit_hour: None,
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
                governance: Default::default(),
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
                governance: Default::default(),
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
        let mut summary = summarize_project_costs(&index, &config);
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
            .filter_map(|c| c.savings_p50_usd_per_month)
            .sum();
        let current = summary.current_cost.monthly_p50.unwrap_or(0.0);
        let potential = summary.potential_savings.monthly_p50.unwrap_or(0.0);
        let post_fix = summary.post_fix_cost.monthly_p50.unwrap_or(0.0);
        assert!(total > 0.0);
        assert!(potential <= current);
        assert!(potential > 0.0);
        assert!(post_fix <= current);
    }

    fn sample_diagnostic(rule_id: &str, line: usize) -> Diagnostic {
        Diagnostic {
            governance: Default::default(),
            rule_id: rule_id.into(),
            severity: Severity::Medium,
            path: PathBuf::from("models/m.sql"),
            line,
            column: 1,
            span: None,
            message: rule_id.into(),
            risk: None,
            suggestion: None,
            confidence: Confidence::High,
            warehouse: None,
            source_provenance: None,
            compiled_line: None,
            compiled_column: None,
            cost_estimate: None,
        }
    }

    #[test]
    fn post_fix_never_exceeds_current_with_many_findings_per_model() {
        let mut config = CostConfig {
            enabled: true,
            ..CostConfig::default()
        };
        config.pricing = crate::config::CostPricingSection {
            model: Some("scan".into()),
            usd_per_tb: Some(6.25),
            usd_per_credit: None,
            tb_per_credit_hour: None,
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
            sample_diagnostic("SQLCOST014", 1),
            sample_diagnostic("SQLCOST018", 2),
            sample_diagnostic("SQLCOST012", 3),
        ];
        let mut summary = summarize_project_costs(&index, &config);
        let ctx = CostAttributionContext {
            config: &config,
            model_index: &index,
            dbt: Some(&dbt),
            features_by_path: &HashMap::new(),
            downstream_counts: &HashMap::new(),
            exposure_counts: &HashMap::new(),
        };
        attribute_findings(&mut diagnostics, &ctx, &mut summary);
        let current = summary.current_cost.monthly_p50.unwrap_or(0.0);
        let post_fix = summary.post_fix_cost.monthly_p50.unwrap_or(0.0);
        let potential = summary.potential_savings.monthly_p50.unwrap_or(0.0);
        assert!(current > 0.0);
        assert!(post_fix <= current);
        assert!(potential >= 0.0);
        assert!(
            potential > 0.0,
            "potential {potential} current {current} post_fix {post_fix}"
        );
    }

    fn linear_chain_dbt(len: usize) -> DbtProject {
        let mut dbt = DbtProject::default();
        for index in 0..len {
            let id = format!("model.chain.m{index}");
            let depends_on = if index == 0 {
                Vec::new()
            } else {
                vec![format!("model.chain.m{}", index - 1)]
            };
            dbt.graph.depends_on.insert(id.clone(), depends_on);
            dbt.models.insert(
                id.clone(),
                DbtModel {
                    unique_id: Some(id),
                    name: format!("m{index}"),
                    ..DbtModel::default()
                },
            );
        }
        dbt
    }

    #[test]
    fn downstream_counts_cap_at_fan_out_saturation() {
        let dbt = linear_chain_dbt(2_000);
        let counts = build_downstream_counts(&dbt);
        assert!(counts.values().all(|count| *count <= 15));
        let root = counts.get("model.chain.m0").copied().unwrap_or(0);
        assert_eq!(root, 15);
        let saturated: f64 = 1.0 + (1.0 + 15.0_f64).log2() * 0.25;
        assert!((saturated - 2.0).abs() < f64::EPSILON);
    }
}
