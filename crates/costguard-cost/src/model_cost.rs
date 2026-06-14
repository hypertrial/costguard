use crate::config::CostConfig;
use crate::estimate::{round_sig2, sum_lognormals, Estimate};
use crate::pricing::price_per_byte;
use crate::volume::{lookup_keys, model_identity, VolumeContext};
use crate::CostInputs;
use costguard_dbt::{DbtModel, DbtProject};
use costguard_diagnostics::CostGrade;
use std::collections::HashMap;
use std::path::PathBuf;

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
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ProjectCostSummary {
    pub project_p10_usd: Option<f64>,
    pub project_p50_usd: Option<f64>,
    pub project_p90_usd: Option<f64>,
    pub project_gb_months: f64,
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
        let monthly_cost = if let Some(price) = price {
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
            },
        ));
    }

    ranked.sort_unstable_by(|(left_score, left), (right_score, right)| {
        right_score
            .total_cmp(left_score)
            .then_with(|| left.model_id.cmp(&right.model_id))
    });
    let top_models = ranked
        .into_iter()
        .take(5)
        .map(|(_, model)| model)
        .collect();

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

    ProjectCostSummary {
        project_p10_usd: project_p10,
        project_p50_usd: project_p50,
        project_p90_usd: project_p90,
        project_gb_months: round_sig2(gb_total),
        savings_p50_usd: 0.0,
        savings_p10_usd: 0.0,
        savings_p90_usd: 0.0,
        savings_gb_months: 0.0,
        grade_a,
        grade_b,
        grade_c,
        model_count: index.models.len(),
        top_models,
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
