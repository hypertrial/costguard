use crate::estimate::{format_usd_interval, round_sig2};
use costguard_diagnostics::CostEstimate;

pub fn format_cost_line(estimate: &CostEstimate) -> String {
    let downstream_note = match (
        estimate.downstream_model_count,
        estimate.downstream_monthly_p50_usd,
    ) {
        (Some(count), Some(usd)) if count > 0 => {
            format!(", downstream {count} models ~${usd:.0}/mo")
        }
        (Some(count), None) if count > 0 => format!(", downstream {count} models"),
        _ => String::new(),
    };
    if let (Some(p10), Some(p50), Some(p90)) = (
        estimate.savings_p10_usd_per_month,
        estimate.savings_p50_usd_per_month,
        estimate.savings_p90_usd_per_month,
    ) {
        let model_note = estimate
            .model_monthly_p50_usd
            .map(|model| format!(", model cost ~${model:.0}/mo"))
            .unwrap_or_default();
        format!(
            "Est. savings: {} (grade {}{model_note}{downstream_note})",
            format_usd_interval(p10, p50, p90),
            estimate.grade
        )
    } else {
        format!(
            "Est. savings index: {:.0} GB-mo (grade {}{downstream_note})",
            round_sig2(estimate.relative_index),
            estimate.grade
        )
    }
}
