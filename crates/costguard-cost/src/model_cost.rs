use crate::config::CostConfig;
use crate::estimate::{annual_from_monthly, round_sig2, sum_lognormals, Estimate};
use crate::pricing::price_per_byte;
use crate::volume::{lookup_keys, model_identity, VolumeContext};
use crate::CostInputs;
use costguard_dbt::{DbtModel, DbtProject};
use costguard_diagnostics::CostGrade;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

#[derive(Debug, Clone, serde::Serialize)]
pub struct CostFigure {
    pub monthly_p10: Option<f64>,
    pub monthly_p50: Option<f64>,
    pub monthly_p90: Option<f64>,
    pub annual_p50: Option<f64>,
    pub basis: String,
}

impl Default for CostFigure {
    fn default() -> Self {
        Self {
            monthly_p10: None,
            monthly_p50: None,
            monthly_p90: None,
            annual_p50: None,
            basis: "none".into(),
        }
    }
}

impl CostFigure {
    pub fn from_estimate(
        estimate: Estimate,
        config: &CostConfig,
        basis: &str,
        has_pricing: bool,
    ) -> Self {
        if !has_pricing {
            return Self {
                monthly_p50: Some(round_sig2(estimate.median())),
                basis: basis.into(),
                ..Default::default()
            };
        }
        let (lo, hi) = estimate.interval(config.interval);
        let p50 = round_sig2(estimate.median());
        Self {
            monthly_p10: Some(round_sig2(lo)),
            monthly_p50: Some(p50),
            monthly_p90: Some(round_sig2(hi)),
            annual_p50: Some(annual_from_monthly(p50)),
            basis: basis.into(),
        }
    }
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct CoverageMetrics {
    pub models_total: usize,
    pub models_with_mapped_spend: usize,
    pub mapped_spend_fraction: f64,
    pub rules_estimated: usize,
    pub rules_unestimated: usize,
    pub observation_age_days: Option<f64>,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct PrCostImpact {
    pub introduced: CostFigure,
    pub avoided: CostFigure,
    pub net: CostFigure,
    pub efficiency: CostFigure,
    pub volume: CostFigure,
    /// Total monthly cost of downstream models affected by changed models (additive advisory).
    pub blast_radius: CostFigure,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct RealizedSavings {
    pub before: CostFigure,
    pub after: CostFigure,
    pub realized: CostFigure,
    pub efficiency: CostFigure,
    pub volume: CostFigure,
}

#[derive(Debug, Clone)]
pub struct ModelCostEntry {
    pub model_id: String,
    pub path: Option<PathBuf>,
    pub bytes: Estimate,
    pub runs_per_month: Estimate,
    pub scan_volume: Estimate,
    pub monthly_cost: Estimate,
    pub gb_months: f64,
    pub grade: CostGrade,
    pub estimation_basis: String,
    pub observation_window: Option<(String, String)>,
    pub observation_age_days: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct ModelCostIndex {
    pub models: HashMap<String, ModelCostEntry>,
    pub by_path: HashMap<PathBuf, String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct TopModelCost {
    pub model_id: String,
    pub path: Option<PathBuf>,
    pub p50_usd_per_month: Option<f64>,
    pub gb_months: f64,
    pub grade: CostGrade,
    pub estimation_basis: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ProjectCostSummary {
    pub project_p10_usd: Option<f64>,
    pub project_p50_usd: Option<f64>,
    pub project_p90_usd: Option<f64>,
    pub project_gb_months: f64,
    pub current_cost: CostFigure,
    pub post_fix_cost: CostFigure,
    pub potential_savings: CostFigure,
    pub coverage: CoverageMetrics,
    pub pr_impact: Option<PrCostImpact>,
    pub realized_savings: Option<RealizedSavings>,
    pub savings_p50_usd: f64,
    pub savings_p10_usd: f64,
    pub savings_p90_usd: f64,
    pub savings_gb_months: f64,
    pub grade_a: usize,
    pub grade_b: usize,
    pub grade_c: usize,
    pub model_count: usize,
    pub top_models: Vec<TopModelCost>,
}

impl Default for ProjectCostSummary {
    fn default() -> Self {
        Self {
            project_p10_usd: None,
            project_p50_usd: None,
            project_p90_usd: None,
            project_gb_months: 0.0,
            current_cost: CostFigure::default(),
            post_fix_cost: CostFigure::default(),
            potential_savings: CostFigure::default(),
            coverage: CoverageMetrics::default(),
            pr_impact: None,
            realized_savings: None,
            savings_p50_usd: 0.0,
            savings_p10_usd: 0.0,
            savings_p90_usd: 0.0,
            savings_gb_months: 0.0,
            grade_a: 0,
            grade_b: 0,
            grade_c: 0,
            model_count: 0,
            top_models: Vec::new(),
        }
    }
}

pub fn build_model_cost_index(
    config: &CostConfig,
    dbt: Option<&DbtProject>,
    inputs: &CostInputs,
) -> ModelCostIndex {
    let ctx = VolumeContext {
        config,
        observations: inputs.observations.as_ref(),
        catalog: inputs.catalog.as_ref(),
        query_history: inputs.query_history.as_ref(),
        dbt,
    };
    let price = price_per_byte(config);
    let mut models = HashMap::new();
    let mut by_path = HashMap::new();

    let model_list: Vec<&DbtModel> = if let Some(project) = dbt {
        let mut models: Vec<&DbtModel> = project.models.values().collect();
        models.sort_by_key(|model| model_identity(model));
        models
    } else {
        Vec::new()
    };

    for model in model_list {
        let volume = ctx.resolve_for_model(model);
        let scan_volume = volume.bytes * volume.runs_per_month;
        let monthly_cost = if let Some(observed) = volume.observed_monthly_cost {
            observed
        } else if let Some(price) = price {
            scan_volume * price
        } else {
            scan_volume
        };
        let gb_months = scan_volume.median() / 1_000_000_000.0;
        let model_id = model_identity(model);
        if let Some(path) = &model.path {
            by_path.insert(path.clone(), model_id.clone());
        }
        models.insert(
            model_id.clone(),
            ModelCostEntry {
                model_id,
                path: model.path.clone(),
                bytes: volume.bytes,
                runs_per_month: volume.runs_per_month,
                scan_volume,
                monthly_cost,
                gb_months,
                grade: volume.grade,
                estimation_basis: volume.estimation_basis,
                observation_window: volume.observation_window,
                observation_age_days: volume.observation_age_days,
            },
        );
    }

    ModelCostIndex { models, by_path }
}

pub fn summarize_project_costs(index: &ModelCostIndex, config: &CostConfig) -> ProjectCostSummary {
    let has_pricing = price_per_byte(config).is_some();
    let mut grade_a = 0usize;
    let mut grade_b = 0usize;
    let mut grade_c = 0usize;
    let mut cost_estimates = Vec::new();
    let mut gb_total = 0.0_f64;
    let mut mapped_spend = 0usize;
    let mut max_age: Option<f64> = None;
    let mut model_ids: Vec<&String> = index.models.keys().collect();
    model_ids.sort();

    let mut ranked: Vec<(f64, TopModelCost)> = Vec::new();
    for model_id in model_ids {
        let entry = &index.models[model_id];
        match entry.grade {
            CostGrade::A => grade_a += 1,
            CostGrade::B => grade_b += 1,
            CostGrade::C => grade_c += 1,
        }
        if entry.grade == CostGrade::A
            || entry.estimation_basis.starts_with("observed-")
            || entry.estimation_basis == "query-history"
        {
            mapped_spend += 1;
        }
        if let Some(age) = entry.observation_age_days {
            max_age = Some(max_age.map_or(age, |current| current.min(age)));
        }
        gb_total += entry.gb_months;
        cost_estimates.push(entry.monthly_cost);
        let score = if has_pricing {
            entry.monthly_cost.median()
        } else {
            entry.gb_months
        };
        ranked.push((
            score,
            TopModelCost {
                model_id: entry.model_id.clone(),
                path: entry.path.clone(),
                p50_usd_per_month: has_pricing.then(|| round_sig2(entry.monthly_cost.median())),
                gb_months: round_sig2(entry.gb_months),
                grade: entry.grade,
                estimation_basis: entry.estimation_basis.clone(),
            },
        ));
    }

    ranked.sort_unstable_by(|(left_score, left), (right_score, right)| {
        right_score
            .total_cmp(left_score)
            .then_with(|| left.model_id.cmp(&right.model_id))
    });
    let top_models = ranked.into_iter().take(5).map(|(_, model)| model).collect();

    let total = sum_lognormals(&cost_estimates);
    let (project_p10, project_p50, project_p90) = if has_pricing {
        let (lo, hi) = total.interval(config.interval);
        (
            Some(round_sig2(lo)),
            Some(round_sig2(total.median())),
            Some(round_sig2(hi)),
        )
    } else {
        (None, None, None)
    };

    let model_count = index.models.len();
    let mapped_fraction = if model_count == 0 {
        0.0
    } else {
        mapped_spend as f64 / model_count as f64
    };

    ProjectCostSummary {
        project_p10_usd: project_p10,
        project_p50_usd: project_p50,
        project_p90_usd: project_p90,
        project_gb_months: round_sig2(gb_total),
        current_cost: CostFigure::from_estimate(total, config, "project-total", has_pricing),
        post_fix_cost: CostFigure::from_estimate(total, config, "project-total", has_pricing),
        potential_savings: CostFigure::default(),
        coverage: CoverageMetrics {
            models_total: model_count,
            models_with_mapped_spend: mapped_spend,
            mapped_spend_fraction: round_sig2(mapped_fraction),
            rules_estimated: 0,
            rules_unestimated: 0,
            observation_age_days: max_age.map(round_sig2),
        },
        pr_impact: None,
        realized_savings: None,
        savings_p50_usd: 0.0,
        savings_p10_usd: 0.0,
        savings_p90_usd: 0.0,
        savings_gb_months: 0.0,
        grade_a,
        grade_b,
        grade_c,
        model_count,
        top_models,
    }
}

pub fn compute_realized_savings(
    before: &crate::observations::ObservationStats,
    after: &crate::observations::ObservationStats,
    config: &CostConfig,
) -> RealizedSavings {
    let has_pricing = price_per_byte(config).is_some();
    let mut before_total = Vec::new();
    let mut after_total = Vec::new();
    let mut efficiency_total = Vec::new();
    let mut volume_total = Vec::new();

    for (model_id, after_cost) in &after.by_model {
        let Some(before_cost) = before.by_model.get(model_id) else {
            continue;
        };
        let before_est = before_cost.monthly_cost_usd.unwrap_or_else(|| {
            Estimate::from_point(before_cost.monthly_bytes.unwrap_or(0.0), Some(0.15))
        });
        let after_est = after_cost.monthly_cost_usd.unwrap_or_else(|| {
            Estimate::from_point(after_cost.monthly_bytes.unwrap_or(0.0), Some(0.15))
        });
        before_total.push(before_est);
        after_total.push(after_est);

        let before_exec = before_cost.monthly_executions.max(1.0);
        let after_exec = after_cost.monthly_executions.max(1.0);
        let before_per_exec = before_est / Estimate::from_point(before_exec, Some(0.1));
        let after_per_exec = after_est / Estimate::from_point(after_exec, Some(0.1));
        let exec_avg = Estimate::from_point((before_exec + after_exec) / 2.0, Some(0.1));
        let efficiency_delta = (before_per_exec - after_per_exec) * exec_avg;
        let volume_delta = (Estimate::from_point(after_exec, Some(0.1))
            - Estimate::from_point(before_exec, Some(0.1)))
            * after_per_exec;
        efficiency_total.push(efficiency_delta);
        volume_total.push(volume_delta);
    }

    let before_sum = sum_lognormals(&before_total);
    let after_sum = sum_lognormals(&after_total);
    let realized = before_sum - after_sum;
    let efficiency = sum_lognormals(&efficiency_total);
    let volume = sum_lognormals(&volume_total);

    RealizedSavings {
        before: CostFigure::from_estimate(before_sum, config, "observed-before", has_pricing),
        after: CostFigure::from_estimate(after_sum, config, "observed-after", has_pricing),
        realized: CostFigure::from_estimate(realized, config, "observed-delta", has_pricing),
        efficiency: CostFigure::from_estimate(efficiency, config, "efficiency-delta", has_pricing),
        volume: CostFigure::from_estimate(volume, config, "volume-delta", has_pricing),
    }
}

pub fn compute_blast_radius(
    index: &ModelCostIndex,
    downstream_ids: &HashMap<String, Vec<String>>,
    changed_paths: &[PathBuf],
    config: &CostConfig,
) -> CostFigure {
    let has_pricing = price_per_byte(config).is_some();
    let mut seen_downstream = HashSet::new();
    let mut costs = Vec::new();

    for path in changed_paths {
        let Some(model_id) = index.by_path.get(path) else {
            continue;
        };
        let Some(ids) = downstream_ids.get(model_id) else {
            continue;
        };
        for id in ids {
            if !seen_downstream.insert(id.clone()) {
                continue;
            }
            if let Some(entry) = index.models.get(id) {
                costs.push(entry.monthly_cost);
            }
        }
    }

    if costs.is_empty() {
        return CostFigure {
            basis: "downstream-blast-radius".into(),
            ..Default::default()
        };
    }

    let total = sum_lognormals(&costs);
    CostFigure::from_estimate(total, config, "downstream-blast-radius", has_pricing)
}

pub fn compute_pr_impact(
    head_summary: &ProjectCostSummary,
    base_summary: &ProjectCostSummary,
    config: &CostConfig,
    blast_radius: CostFigure,
) -> PrCostImpact {
    let has_pricing = price_per_byte(config).is_some();

    let head_current_p50 = head_summary.current_cost.monthly_p50.unwrap_or(0.0);
    let base_current_p50 = base_summary.current_cost.monthly_p50.unwrap_or(0.0);
    let head_savings_p50 = head_summary.potential_savings.monthly_p50.unwrap_or(0.0);
    let base_savings_p50 = base_summary.potential_savings.monthly_p50.unwrap_or(0.0);

    let current_delta = head_current_p50 - base_current_p50;
    let savings_delta = head_savings_p50 - base_savings_p50;
    let volume_delta = current_delta - savings_delta;

    let introduced_est = Estimate::from_point(current_delta.max(0.0), Some(0.15));
    let avoided_est = Estimate::from_point((-current_delta).max(0.0), Some(0.15));
    let net_est = Estimate::from_point(current_delta, Some(0.15));
    let efficiency_est = Estimate::from_point(savings_delta, Some(0.2));
    let volume_est = Estimate::from_point(volume_delta, Some(0.2));

    PrCostImpact {
        introduced: CostFigure::from_estimate(introduced_est, config, "pr-introduced", has_pricing),
        avoided: CostFigure::from_estimate(avoided_est, config, "pr-avoided", has_pricing),
        net: CostFigure::from_estimate(net_est, config, "pr-net", has_pricing),
        efficiency: CostFigure::from_estimate(efficiency_est, config, "pr-efficiency", has_pricing),
        volume: CostFigure::from_estimate(volume_est, config, "pr-volume", has_pricing),
        blast_radius,
    }
}

pub fn lookup_model_entry<'a>(
    index: &'a ModelCostIndex,
    path: &std::path::Path,
    model: Option<&DbtModel>,
) -> Option<&'a ModelCostEntry> {
    if let Some(model_id) = index.by_path.get(path) {
        return index.models.get(model_id);
    }
    if let Some(model) = model {
        let id = model_identity(model);
        if let Some(entry) = index.models.get(&id) {
            return Some(entry);
        }
        for key in lookup_keys(model) {
            if let Some(entry) = index.models.get(&key) {
                return Some(entry);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::CostPricingSection;
    use costguard_dbt::DbtModel;

    #[test]
    fn deduplicated_project_total_sums_models_once() {
        let mut config = CostConfig {
            enabled: true,
            ..CostConfig::default()
        };
        config.pricing = CostPricingSection {
            model: Some("scan".into()),
            usd_per_tb: Some(5.0),
            usd_per_credit: None,
            tb_per_credit_hour: None,
        };
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
                ..DbtModel::default()
            },
        );
        let index = build_model_cost_index(&config, Some(&dbt), &CostInputs::default());
        let summary = summarize_project_costs(&index, &config);
        assert_eq!(summary.model_count, 2);
        assert!(summary.project_p50_usd.unwrap_or(0.0) > 0.0);
        assert!(summary.current_cost.monthly_p50.unwrap_or(0.0) > 0.0);
    }

    #[test]
    fn top_models_order_is_stable_when_costs_tie() {
        let mut config = CostConfig {
            enabled: true,
            ..CostConfig::default()
        };
        config.pricing = CostPricingSection {
            model: Some("scan".into()),
            usd_per_tb: Some(5.0),
            usd_per_credit: None,
            tb_per_credit_hour: None,
        };
        let mut dbt = DbtProject::default();
        for name in ["alpha", "beta", "gamma", "delta", "epsilon", "zeta"] {
            dbt.models.insert(
                name.into(),
                DbtModel {
                    name: name.into(),
                    path: Some(PathBuf::from(format!("models/{name}.sql"))),
                    ..DbtModel::default()
                },
            );
        }
        let index = build_model_cost_index(&config, Some(&dbt), &CostInputs::default());
        let first = summarize_project_costs(&index, &config);
        let second = summarize_project_costs(&index, &config);
        assert_eq!(first.top_models, second.top_models);
        assert_eq!(first.top_models.len(), 5);
        let model_ids: Vec<String> = first
            .top_models
            .iter()
            .map(|entry| entry.model_id.clone())
            .collect();
        let mut sorted_ids = model_ids.clone();
        sorted_ids.sort();
        assert_eq!(model_ids, sorted_ids);
    }
}
