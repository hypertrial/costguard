use crate::config::CostConfig;
use crate::estimate::{annual_from_monthly, round_sig2, Estimate};
use crate::pricing::price_per_byte;
use crate::units::{
    price_bytes, sum_bytes, sum_usd, BytesPerMonthEstimate, UnitlessEstimate, UsdPerMonthEstimate,
};
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
    pub gb_months_p50: Option<f64>,
    pub basis: String,
}

impl Default for CostFigure {
    fn default() -> Self {
        Self {
            monthly_p10: None,
            monthly_p50: None,
            monthly_p90: None,
            annual_p50: None,
            gb_months_p50: None,
            basis: "none".into(),
        }
    }
}

impl CostFigure {
    pub fn from_estimates(
        usd: Option<Estimate>,
        bytes: Option<Estimate>,
        config: &CostConfig,
        basis: &str,
    ) -> Self {
        Self::from_typed(
            usd.map(UsdPerMonthEstimate::from_raw),
            bytes.map(BytesPerMonthEstimate::from_raw),
            config,
            basis,
        )
    }

    pub(crate) fn from_typed(
        usd: Option<UsdPerMonthEstimate>,
        bytes: Option<BytesPerMonthEstimate>,
        config: &CostConfig,
        basis: &str,
    ) -> Self {
        let (monthly_p10, monthly_p50, monthly_p90, annual_p50) =
            usd.map_or((None, None, None, None), |estimate| {
                let (lo, hi) = estimate.interval(config.interval);
                let p50 = round_sig2(estimate.median());
                (
                    Some(round_sig2(lo)),
                    Some(p50),
                    Some(round_sig2(hi)),
                    Some(annual_from_monthly(p50)),
                )
            });
        Self {
            monthly_p10,
            monthly_p50,
            monthly_p90,
            annual_p50,
            gb_months_p50: bytes.map(|estimate| round_sig2(estimate.median() / 1e9)),
            basis: basis.into(),
        }
    }

    fn from_points(usd: Option<f64>, gb_months: Option<f64>, basis: &str) -> Self {
        Self {
            monthly_p50: usd.map(round_sig2),
            annual_p50: usd.map(annual_from_monthly),
            gb_months_p50: gb_months.map(round_sig2),
            basis: basis.into(),
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct CoverageMetrics {
    pub models_total: usize,
    pub models_with_mapped_spend: usize,
    pub mapped_spend_fraction: f64,
    pub models_with_usd: usize,
    pub usd_coverage_fraction: f64,
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
    pub monthly_cost_usd: Option<Estimate>,
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
    pub savings_p50_usd: Option<f64>,
    pub savings_p10_usd: Option<f64>,
    pub savings_p90_usd: Option<f64>,
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
            savings_p50_usd: None,
            savings_p10_usd: None,
            savings_p90_usd: None,
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
        let scan_volume =
            BytesPerMonthEstimate::from_bytes_and_runs(volume.bytes, volume.runs_per_month);
        let monthly_cost_usd = volume
            .observed_monthly_cost_usd
            .map(UsdPerMonthEstimate::from_raw)
            .or_else(|| price.map(|price| price_bytes(scan_volume, price)));
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
                scan_volume: scan_volume.raw(),
                monthly_cost_usd: monthly_cost_usd.map(UsdPerMonthEstimate::raw),
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
    let mut grade_a = 0usize;
    let mut grade_b = 0usize;
    let mut grade_c = 0usize;
    let mut cost_estimates: Vec<UsdPerMonthEstimate> = Vec::new();
    let mut mapped_spend = 0usize;
    let mut models_with_usd = 0usize;
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
            max_age = Some(max_age.map_or(age, |current| current.max(age)));
        }
        if let Some(cost) = entry.monthly_cost_usd {
            cost_estimates.push(UsdPerMonthEstimate::from_raw(cost));
            models_with_usd += 1;
        }
        ranked.push((
            entry.gb_months,
            TopModelCost {
                model_id: entry.model_id.clone(),
                path: entry.path.clone(),
                p50_usd_per_month: entry.monthly_cost_usd.map(|cost| round_sig2(cost.median())),
                gb_months: round_sig2(entry.gb_months),
                grade: entry.grade,
                estimation_basis: entry.estimation_basis.clone(),
            },
        ));
    }

    let full_usd_coverage = !index.models.is_empty() && models_with_usd == index.models.len();
    if full_usd_coverage {
        for (score, model) in &mut ranked {
            *score = model.p50_usd_per_month.unwrap_or(0.0);
        }
    }
    ranked.sort_unstable_by(|(left_score, left), (right_score, right)| {
        right_score
            .total_cmp(left_score)
            .then_with(|| left.model_id.cmp(&right.model_id))
    });
    let top_models = ranked.into_iter().take(5).map(|(_, model)| model).collect();

    let total_usd = sum_usd(&cost_estimates);
    let total_bytes = sum_bytes(
        &index
            .models
            .values()
            .map(|entry| BytesPerMonthEstimate::from_raw(entry.scan_volume))
            .collect::<Vec<_>>(),
    );
    let (project_p10, project_p50, project_p90) = if let Some(total) = total_usd {
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
    let usd_fraction = if model_count == 0 {
        0.0
    } else {
        models_with_usd as f64 / model_count as f64
    };
    let total_basis = if models_with_usd > 0 && models_with_usd < model_count {
        "mapped-project-total"
    } else {
        "project-total"
    };

    ProjectCostSummary {
        project_p10_usd: project_p10,
        project_p50_usd: project_p50,
        project_p90_usd: project_p90,
        project_gb_months: total_bytes
            .map(|estimate| round_sig2(estimate.median() / 1e9))
            .unwrap_or(0.0),
        current_cost: CostFigure::from_typed(total_usd, total_bytes, config, total_basis),
        post_fix_cost: CostFigure::from_typed(total_usd, total_bytes, config, total_basis),
        potential_savings: CostFigure::default(),
        coverage: CoverageMetrics {
            models_total: model_count,
            models_with_mapped_spend: mapped_spend,
            mapped_spend_fraction: round_sig2(mapped_fraction),
            models_with_usd,
            usd_coverage_fraction: usd_fraction,
            rules_estimated: 0,
            rules_unestimated: 0,
            observation_age_days: max_age.map(round_sig2),
        },
        pr_impact: None,
        realized_savings: None,
        savings_p50_usd: None,
        savings_p10_usd: None,
        savings_p90_usd: None,
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
    let mut before_usd: Vec<UsdPerMonthEstimate> = Vec::new();
    let mut after_usd: Vec<UsdPerMonthEstimate> = Vec::new();
    let mut efficiency_usd: Vec<UsdPerMonthEstimate> = Vec::new();
    let mut volume_usd: Vec<UsdPerMonthEstimate> = Vec::new();
    let mut before_bytes: Vec<BytesPerMonthEstimate> = Vec::new();
    let mut after_bytes: Vec<BytesPerMonthEstimate> = Vec::new();
    let mut efficiency_bytes: Vec<BytesPerMonthEstimate> = Vec::new();
    let mut volume_bytes: Vec<BytesPerMonthEstimate> = Vec::new();

    for (model_id, after_cost) in &after.by_model {
        let Some(before_cost) = before.by_model.get(model_id) else {
            continue;
        };
        let before_exec = before_cost.monthly_executions.max(1.0);
        let after_exec = after_cost.monthly_executions.max(1.0);
        let before_executions = UnitlessEstimate::from_point(before_exec, 0.1);
        let after_executions = UnitlessEstimate::from_point(after_exec, 0.1);
        let exec_avg = UnitlessEstimate::from_point((before_exec + after_exec) / 2.0, 0.1);
        let execution_delta = UnitlessEstimate::from_raw(
            Estimate::from_point(after_exec, Some(0.1))
                - Estimate::from_point(before_exec, Some(0.1)),
        );
        if let (Some(before_est), Some(after_est)) =
            (before_cost.monthly_cost_usd, after_cost.monthly_cost_usd)
        {
            let before_est = UsdPerMonthEstimate::from_raw(before_est);
            let after_est = UsdPerMonthEstimate::from_raw(after_est);
            before_usd.push(before_est);
            after_usd.push(after_est);
            efficiency_usd.push(UsdPerMonthEstimate::efficiency_change(
                before_est,
                after_est,
                before_executions,
                after_executions,
                exec_avg,
            ));
            volume_usd.push(UsdPerMonthEstimate::volume_change(
                after_est,
                after_executions,
                execution_delta,
            ));
        }
        if let (Some(before_value), Some(after_value)) =
            (before_cost.monthly_bytes, after_cost.monthly_bytes)
        {
            let before_est =
                BytesPerMonthEstimate::from_raw(Estimate::from_point(before_value, Some(0.15)));
            let after_est =
                BytesPerMonthEstimate::from_raw(Estimate::from_point(after_value, Some(0.15)));
            before_bytes.push(before_est);
            after_bytes.push(after_est);
            efficiency_bytes.push(BytesPerMonthEstimate::efficiency_change(
                before_est,
                after_est,
                before_executions,
                after_executions,
                exec_avg,
            ));
            volume_bytes.push(BytesPerMonthEstimate::volume_change(
                after_est,
                after_executions,
                execution_delta,
            ));
        }
    }

    let before_usd_sum = sum_usd(&before_usd);
    let after_usd_sum = sum_usd(&after_usd);
    let before_bytes_sum = sum_bytes(&before_bytes);
    let after_bytes_sum = sum_bytes(&after_bytes);

    RealizedSavings {
        before: CostFigure::from_typed(before_usd_sum, before_bytes_sum, config, "observed-before"),
        after: CostFigure::from_typed(after_usd_sum, after_bytes_sum, config, "observed-after"),
        realized: CostFigure::from_points(
            before_usd_sum
                .zip(after_usd_sum)
                .map(|(before, after)| before.median() - after.median()),
            before_bytes_sum
                .zip(after_bytes_sum)
                .map(|(before, after)| (before.median() - after.median()) / 1e9),
            "observed-delta",
        ),
        efficiency: CostFigure::from_typed(
            sum_usd(&efficiency_usd),
            sum_bytes(&efficiency_bytes),
            config,
            "efficiency-delta",
        ),
        volume: CostFigure::from_typed(
            sum_usd(&volume_usd),
            sum_bytes(&volume_bytes),
            config,
            "volume-delta",
        ),
    }
}

pub fn compute_blast_radius(
    index: &ModelCostIndex,
    downstream_ids: &HashMap<String, Vec<String>>,
    changed_paths: &[PathBuf],
    config: &CostConfig,
) -> CostFigure {
    let mut seen_downstream = HashSet::new();
    let mut costs_usd: Vec<UsdPerMonthEstimate> = Vec::new();
    let mut volumes: Vec<BytesPerMonthEstimate> = Vec::new();

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
                if let Some(cost) = entry.monthly_cost_usd {
                    costs_usd.push(UsdPerMonthEstimate::from_raw(cost));
                }
                volumes.push(BytesPerMonthEstimate::from_raw(entry.scan_volume));
            }
        }
    }

    if volumes.is_empty() {
        return CostFigure {
            basis: "downstream-blast-radius".into(),
            ..Default::default()
        };
    }

    CostFigure::from_typed(
        sum_usd(&costs_usd),
        sum_bytes(&volumes),
        config,
        "downstream-blast-radius",
    )
}

pub fn compute_pr_impact(
    head_summary: &ProjectCostSummary,
    base_summary: &ProjectCostSummary,
    _config: &CostConfig,
    blast_radius: CostFigure,
) -> PrCostImpact {
    let usd_comparable = head_summary.coverage.models_total > 0
        && head_summary.coverage.models_with_usd == head_summary.coverage.models_total
        && base_summary.coverage.models_total > 0
        && base_summary.coverage.models_with_usd == base_summary.coverage.models_total;
    let usd_delta = if usd_comparable {
        head_summary
            .current_cost
            .monthly_p50
            .zip(base_summary.current_cost.monthly_p50)
            .map(|(head, base)| head - base)
    } else {
        None
    };
    let gb_delta = head_summary
        .current_cost
        .gb_months_p50
        .zip(base_summary.current_cost.gb_months_p50)
        .map(|(head, base)| head - base);
    let usd_savings_delta = usd_comparable.then(|| {
        head_summary.potential_savings.monthly_p50.unwrap_or(0.0)
            - base_summary.potential_savings.monthly_p50.unwrap_or(0.0)
    });
    let gb_savings_delta = head_summary
        .current_cost
        .gb_months_p50
        .zip(base_summary.current_cost.gb_months_p50)
        .map(|_| {
            head_summary.potential_savings.gb_months_p50.unwrap_or(0.0)
                - base_summary.potential_savings.gb_months_p50.unwrap_or(0.0)
        });

    PrCostImpact {
        introduced: CostFigure::from_points(
            usd_delta.map(|v| v.max(0.0)),
            gb_delta.map(|v| v.max(0.0)),
            "pr-introduced",
        ),
        avoided: CostFigure::from_points(
            usd_delta.map(|v| (-v).max(0.0)),
            gb_delta.map(|v| (-v).max(0.0)),
            "pr-avoided",
        ),
        net: CostFigure::from_points(usd_delta, gb_delta, "pr-net"),
        efficiency: CostFigure::from_points(usd_savings_delta, gb_savings_delta, "pr-efficiency"),
        volume: CostFigure::from_points(
            usd_delta
                .zip(usd_savings_delta)
                .map(|(current, savings)| current - savings),
            gb_delta
                .zip(gb_savings_delta)
                .map(|(current, savings)| current - savings),
            "pr-volume",
        ),
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

    #[test]
    fn unpriced_project_reports_volume_without_usd() {
        let config = CostConfig {
            enabled: true,
            ..CostConfig::default()
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
        let index = build_model_cost_index(&config, Some(&dbt), &CostInputs::default());
        let summary = summarize_project_costs(&index, &config);
        assert!(summary.project_p50_usd.is_none());
        assert!(summary.current_cost.monthly_p50.is_none());
        assert!(summary.current_cost.gb_months_p50.unwrap_or(0.0) > 0.0);
        assert_eq!(summary.coverage.models_with_usd, 0);
        assert_eq!(summary.coverage.usd_coverage_fraction, 0.0);
    }

    #[test]
    fn coverage_reports_the_stalest_observation_age() {
        let estimate = Estimate::from_point(1_000_000_000.0, Some(0.1));
        let entry = |model_id: &str, age| ModelCostEntry {
            model_id: model_id.into(),
            path: None,
            bytes: estimate,
            runs_per_month: Estimate::from_point(1.0, Some(0.1)),
            scan_volume: estimate,
            monthly_cost_usd: None,
            gb_months: 1.0,
            grade: CostGrade::A,
            estimation_basis: "observed-bytes".into(),
            observation_window: None,
            observation_age_days: Some(age),
        };
        let index = ModelCostIndex {
            models: HashMap::from([
                ("recent".into(), entry("recent", 2.0)),
                ("stale".into(), entry("stale", 10.0)),
            ]),
            by_path: HashMap::new(),
        };

        let summary = summarize_project_costs(&index, &CostConfig::default());

        assert_eq!(summary.coverage.observation_age_days, Some(10.0));
    }

    #[test]
    fn partial_observed_usd_is_mapped_without_hiding_full_volume() {
        let config = CostConfig {
            enabled: true,
            ..CostConfig::default()
        };
        let mut dbt = DbtProject::default();
        for name in ["a", "b"] {
            dbt.models.insert(
                name.into(),
                DbtModel {
                    name: name.into(),
                    path: Some(PathBuf::from(format!("models/{name}.sql"))),
                    ..DbtModel::default()
                },
            );
        }
        let observations = crate::observations::parse_observations_text(
            r#"{
                "schema_version": 1,
                "organization": "acme",
                "repository": "acme/warehouse",
                "currency": "USD",
                "generated_at": "2026-06-15T00:00:00Z",
                "provenance": "test",
                "observations": [{
                    "model_id": "a",
                    "window_start": "2026-06-01T00:00:00Z",
                    "window_end": "2026-07-01T00:00:00Z",
                    "executions": 10,
                    "cost_usd": 42.0
                }]
            }"#,
        )
        .unwrap();
        let inputs = CostInputs {
            observations: Some(observations),
            ..CostInputs::default()
        };
        let index = build_model_cost_index(&config, Some(&dbt), &inputs);
        let observed = &index.models["a"];
        assert!(observed.monthly_cost_usd.is_some());
        assert!(observed.estimation_basis.contains("size-prior"));
        assert!(observed.scan_volume.median() > 1_000_000_000.0);
        let summary = summarize_project_costs(&index, &config);
        assert!(summary.project_p50_usd.unwrap_or(0.0) > 0.0);
        assert!(summary.project_gb_months > 0.0);
        assert_eq!(summary.coverage.models_with_usd, 1);
        assert_eq!(summary.coverage.usd_coverage_fraction, 0.5);
        assert!(summary
            .top_models
            .iter()
            .any(|model| model.p50_usd_per_month.is_none()));
    }

    #[test]
    fn partial_usd_coverage_uses_only_volume_for_pr_delta() {
        let summary = |usd: f64, gb: f64| ProjectCostSummary {
            current_cost: CostFigure {
                monthly_p50: Some(usd),
                gb_months_p50: Some(gb),
                ..CostFigure::default()
            },
            coverage: CoverageMetrics {
                models_total: 2,
                models_with_usd: 1,
                usd_coverage_fraction: 0.5,
                ..CoverageMetrics::default()
            },
            ..ProjectCostSummary::default()
        };
        let impact = compute_pr_impact(
            &summary(100.0, 12.0),
            &summary(80.0, 10.0),
            &CostConfig::default(),
            CostFigure::default(),
        );

        assert!(impact.net.monthly_p50.is_none());
        assert_eq!(impact.net.gb_months_p50, Some(2.0));
    }
}
