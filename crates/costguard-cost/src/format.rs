use crate::estimate::{format_usd_interval, round_sig2};
use costguard_diagnostics::CostEstimate;

pub fn format_cost_line(estimate: &CostEstimate) -> String {
    if let (Some(p10), Some(p50), Some(p90)) = (
        estimate.p10_usd_per_month,
        estimate.p50_usd_per_month,
        estimate.p90_usd_per_month,
    ) {
        format!(
            "Est. cost: {} (grade {})",
            format_usd_interval(p10, p50, p90),
            estimate.grade
        )
    } else {
        format!(
            "Est. relative cost index: {:.0} (grade {})",
            round_sig2(estimate.relative_index),
            estimate.grade
        )
    }
}
