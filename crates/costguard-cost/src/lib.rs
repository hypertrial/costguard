mod annotate;
mod attribution;
mod catalog;
mod config;
mod estimate;
mod format;
mod model_cost;
mod multipliers;
mod pricing;
mod query_history;
mod volume;

pub use annotate::{
    annotate_diagnostics, run_cost_analysis, total_p50_usd_per_month, total_savings_gb_months,
    CostAnalysisResult, CostInputs,
};
pub use attribution::{
    build_downstream_counts, build_exposure_counts, summarize_features, ModelFeatureSummary,
};
pub use catalog::CatalogStats;
pub use config::{
    CostConfig, CostInputsSection, CostModelOverride, CostPricingSection, CostRuleOverride,
    CostSection, CostSourceOverride, TableSizeClass,
};
pub use estimate::{
    excess_multiplier, format_usd, format_usd_interval, gb_months_from_bytes_runs, round_sig2,
    sum_lognormals, Estimate,
};
pub use format::format_cost_line;
pub use model_cost::{
    build_model_cost_index, lookup_model_entry, summarize_project_costs, ModelCostEntry,
    ModelCostIndex, ProjectCostSummary, TopModelCost,
};
pub use query_history::QueryHistoryStats;
