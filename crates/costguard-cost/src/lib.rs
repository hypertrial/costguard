//! Advisory cost estimation.
//!
//! Applies a lognormal cost model to diagnostics using catalog stats,
//! query-history imports, and rule-specific multipliers. Estimates are
//! local and do not require warehouse credentials.

mod annotate;
mod attribution;
mod catalog;
mod config;
mod csv;
mod estimate;
mod format;
mod import;
mod model_cost;
mod multipliers;
mod observations;
mod pricing;
mod query_history;
mod volume;

pub use annotate::{
    run_cost_analysis, total_p50_usd_per_month, total_savings_gb_months, CostAnalysisResult,
    CostInputs,
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
    annual_from_monthly, combined_multiplier, excess_multiplier, format_usd, format_usd_interval,
    gb_months_from_bytes_runs, round_sig2, savings_fraction, sum_lognormals, Estimate,
};
pub use format::format_cost_line;
pub use import::{
    normalize_cost_export, validate_cost_bundle, CostExportFormat, NormalizeCostOptions,
};
pub use model_cost::{
    build_model_cost_index, compute_pr_impact, compute_realized_savings, lookup_model_entry,
    summarize_project_costs, CostFigure, CoverageMetrics, ModelCostEntry, ModelCostIndex,
    PrCostImpact, ProjectCostSummary, RealizedSavings, TopModelCost,
};
pub use multipliers::{
    is_cost_bearing_rule, is_infrastructure_rule, rule_multiplier, unestimated_reason,
};
pub use observations::{load_observations, ObservationStats, ObservedModelCost};
pub use query_history::QueryHistoryStats;
