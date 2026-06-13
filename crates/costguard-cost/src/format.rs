use crate::estimate::{format_usd_interval, round_sig2};
use costguard_diagnostics::CostEstimate;

pub fn format_cost_line(estimate: &CostEstimate) -> String {
    if let (Some(p10), Some(p50), Some(p90)) = (
        estimate.savings_p10_usd_per_month,
        estimate.savings_p50_usd_per_month,
        estimate.savings_p90_usd_per_month,
    ) {
        let model_note = estimate
            .model_monthly_p50_usd
            .map(|model| format!(", model ~${model:.0}/mo"))
            .unwrap_or_default();
        format!(
            "Est. savings: {} (grade {}{model_note})",
            format_usd_interval(p10, p50, p90),
            estimate.grade
        )
    } else {
        format!(
            "Est. savings index: {:.0} GB-mo (grade {})",
            round_sig2(estimate.relative_index),
            estimate.grade
        )
    }
}
