use crate::config::{range_or_point_to_estimate, CostConfig, CostPricingSection};
use crate::Estimate;

const BYTES_PER_TB: f64 = 1_000_000_000_000.0;

pub fn price_per_byte(config: &CostConfig) -> Option<Estimate> {
    let model = config.pricing.model.as_deref()?;
    match model.to_ascii_lowercase().as_str() {
        "scan" => {
            let usd_per_tb = config.pricing.usd_per_tb.unwrap_or(6.25);
            Some(Estimate::from_point(usd_per_tb / BYTES_PER_TB, Some(0.05)))
        }
        "compute" => {
            let usd_per_credit = config.pricing.usd_per_credit.unwrap_or(3.0);
            let tb_per_hour = config
                .pricing
                .tb_per_credit_hour
                .as_ref()
                .map(|spec| range_or_point_to_estimate(spec, Some(0.4)))
                .unwrap_or_else(|| Estimate::from_range(0.5, 2.0));
            let bytes_per_credit_hour = Estimate::from_point(
                tb_per_hour.median() * BYTES_PER_TB,
                Some(tb_per_hour.sigma / tb_per_hour.mu),
            );
            Some(Estimate::from_point(usd_per_credit, Some(0.1)) / bytes_per_credit_hour)
        }
        _ => None,
    }
}

pub fn pricing_label(pricing: &CostPricingSection) -> String {
    match pricing.model.as_deref().map(str::to_ascii_lowercase) {
        Some(model) if model == "scan" => {
            format!("${}/TB scan", pricing.usd_per_tb.unwrap_or(6.25))
        }
        Some(model) if model == "compute" => {
            format!("${}/credit compute", pricing.usd_per_credit.unwrap_or(3.0))
        }
        _ => "relative index".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::CostPricingSection;

    #[test]
    fn scan_pricing_positive() {
        let config = CostConfig {
            pricing: CostPricingSection {
                model: Some("scan".into()),
                usd_per_tb: Some(6.25),
                usd_per_credit: None,
                tb_per_credit_hour: None,
            },
            ..CostConfig::default()
        };
        let price = price_per_byte(&config).unwrap();
        assert!(price.median() > 0.0);
    }
}
