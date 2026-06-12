use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CostSection {
    pub enabled: Option<bool>,
    pub interval: Option<f64>,
    pub default_runs_per_month: Option<f64>,
    pub default_table_size: Option<String>,
    pub fail_on_monthly_delta: Option<f64>,
    pub incremental_fraction: Option<f64>,
    pub pricing: Option<CostPricingSection>,
    pub inputs: Option<CostInputsSection>,
    #[serde(default)]
    pub sources: HashMap<String, CostSourceOverride>,
    #[serde(default)]
    pub models: HashMap<String, CostModelOverride>,
    #[serde(default)]
    pub rules: HashMap<String, CostRuleOverride>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CostPricingSection {
    pub model: Option<String>,
    pub usd_per_tb: Option<f64>,
    pub usd_per_credit: Option<f64>,
    pub tb_per_credit_hour: Option<RangeOrPoint>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CostInputsSection {
    pub catalog: Option<PathBuf>,
    pub query_history: Option<PathBuf>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CostSourceOverride {
    pub bytes: Option<BytesSpec>,
    pub rows: Option<RangeOrPoint>,
    pub avg_row_bytes: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CostModelOverride {
    pub runs_per_month: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CostRuleOverride {
    pub multiplier: Option<RangeOrPoint>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum BytesSpec {
    Text(String),
    Number(f64),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum RangeOrPoint {
    Point(f64),
    Range { p10: f64, p90: f64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableSizeClass {
    Small,
    Medium,
    Large,
    Xlarge,
}

impl TableSizeClass {
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "small" | "s" => Self::Small,
            "large" | "l" => Self::Large,
            "xlarge" | "xl" | "extra_large" => Self::Xlarge,
            _ => Self::Medium,
        }
    }

    pub fn default_bytes(self) -> f64 {
        match self {
            Self::Small => 1_000_000_000.0,      // 1 GB
            Self::Medium => 50_000_000_000.0,    // 50 GB
            Self::Large => 500_000_000_000.0,    // 500 GB
            Self::Xlarge => 5_000_000_000_000.0, // 5 TB
        }
    }
}

#[derive(Debug, Clone)]
pub struct CostConfig {
    pub enabled: bool,
    pub interval: f64,
    pub default_runs_per_month: f64,
    pub default_table_size: TableSizeClass,
    pub fail_on_monthly_delta: Option<f64>,
    pub incremental_fraction: f64,
    pub pricing: CostPricingSection,
    pub inputs: CostInputsSection,
    pub sources: HashMap<String, CostSourceOverride>,
    pub models: HashMap<String, CostModelOverride>,
    pub rules: HashMap<String, CostRuleOverride>,
}

impl Default for CostConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            interval: 0.80,
            default_runs_per_month: 30.0,
            default_table_size: TableSizeClass::Medium,
            fail_on_monthly_delta: None,
            incremental_fraction: 0.05,
            pricing: CostPricingSection {
                model: None,
                usd_per_tb: None,
                usd_per_credit: None,
                tb_per_credit_hour: None,
            },
            inputs: CostInputsSection {
                catalog: None,
                query_history: None,
            },
            sources: HashMap::new(),
            models: HashMap::new(),
            rules: HashMap::new(),
        }
    }
}

impl CostConfig {
    pub fn from_section(section: CostSection) -> Self {
        let default_table_size = section
            .default_table_size
            .as_deref()
            .map(TableSizeClass::parse)
            .unwrap_or(TableSizeClass::Medium);
        Self {
            enabled: section.enabled.unwrap_or(true),
            interval: section.interval.unwrap_or(0.80).clamp(0.5, 0.99),
            default_runs_per_month: section.default_runs_per_month.unwrap_or(30.0).max(0.0),
            default_table_size,
            fail_on_monthly_delta: section.fail_on_monthly_delta,
            incremental_fraction: section
                .incremental_fraction
                .unwrap_or(0.05)
                .clamp(0.001, 1.0),
            pricing: section.pricing.unwrap_or(CostPricingSection {
                model: None,
                usd_per_tb: None,
                usd_per_credit: None,
                tb_per_credit_hour: None,
            }),
            inputs: section.inputs.unwrap_or(CostInputsSection {
                catalog: None,
                query_history: None,
            }),
            sources: section.sources,
            models: section.models,
            rules: section
                .rules
                .into_iter()
                .map(|(k, v)| (k.to_ascii_uppercase(), v))
                .collect(),
        }
    }

    pub fn dollar_mode(&self) -> bool {
        self.pricing.model.is_some()
    }
}

pub fn range_or_point_to_estimate(spec: &RangeOrPoint, default_cv: Option<f64>) -> crate::Estimate {
    match spec {
        RangeOrPoint::Point(value) => crate::Estimate::from_point(*value, default_cv),
        RangeOrPoint::Range { p10, p90 } => crate::Estimate::from_range(*p10, *p90),
    }
}

pub fn parse_bytes_spec(spec: &BytesSpec) -> anyhow::Result<f64> {
    match spec {
        BytesSpec::Number(n) => Ok(*n),
        BytesSpec::Text(text) => parse_human_bytes(text),
    }
}

pub fn parse_human_bytes(text: &str) -> anyhow::Result<f64> {
    let trimmed = text.trim().to_ascii_uppercase().replace('_', "");
    let trimmed = trimmed.replace(' ', "");
    let units: [(&str, f64); 5] = [
        ("TB", 1_000_000_000_000.0),
        ("GB", 1_000_000_000.0),
        ("MB", 1_000_000.0),
        ("KB", 1_000.0),
        ("B", 1.0),
    ];
    for (suffix, factor) in units {
        if let Some(num) = trimmed.strip_suffix(suffix) {
            let value: f64 = num
                .parse()
                .map_err(|_| anyhow::anyhow!("invalid bytes '{text}'"))?;
            return Ok(value * factor);
        }
    }
    trimmed
        .parse::<f64>()
        .map_err(|_| anyhow::anyhow!("invalid bytes '{text}'"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_human_bytes() {
        assert_eq!(parse_human_bytes("12TB").unwrap(), 12_000_000_000_000.0);
        assert_eq!(parse_human_bytes("500 GB").unwrap(), 500_000_000_000.0);
    }
}
