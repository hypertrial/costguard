use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Warehouse {
    #[default]
    Generic,
    Snowflake,
    BigQuery,
    Trino,
    Databricks,
    Redshift,
    Postgres,
    DuckDb,
}

impl Warehouse {
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "snowflake" => Self::Snowflake,
            "bigquery" => Self::BigQuery,
            "trino" | "presto" => Self::Trino,
            "databricks" => Self::Databricks,
            "redshift" => Self::Redshift,
            "postgres" | "postgresql" => Self::Postgres,
            "duckdb" => Self::DuckDb,
            _ => Self::Generic,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CostSection {
    pub enabled: Option<bool>,
    pub interval: Option<f64>,
    pub default_runs_per_month: Option<f64>,
    pub default_table_size: Option<String>,
    pub fail_on_monthly_delta: Option<f64>,
    pub fail_on_monthly_delta_gb: Option<f64>,
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
    pub observations: Option<PathBuf>,
    pub observations_before: Option<PathBuf>,
    pub observations_after: Option<PathBuf>,
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
    pub fn parse(value: &str) -> anyhow::Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "small" | "s" => Ok(Self::Small),
            "medium" | "m" => Ok(Self::Medium),
            "large" | "l" => Ok(Self::Large),
            "xlarge" | "xl" | "extra_large" => Ok(Self::Xlarge),
            other => anyhow::bail!("unknown cost table size class '{other}'"),
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
    pub warehouse: Warehouse,
    pub interval: f64,
    pub default_runs_per_month: f64,
    pub default_table_size: TableSizeClass,
    pub fail_on_monthly_delta: Option<f64>,
    pub fail_on_monthly_delta_gb: Option<f64>,
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
            warehouse: Warehouse::Generic,
            interval: 0.80,
            default_runs_per_month: 30.0,
            default_table_size: TableSizeClass::Medium,
            fail_on_monthly_delta: None,
            fail_on_monthly_delta_gb: None,
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
                observations: None,
                observations_before: None,
                observations_after: None,
            },
            sources: HashMap::new(),
            models: HashMap::new(),
            rules: HashMap::new(),
        }
    }
}

impl CostConfig {
    pub fn from_section(section: CostSection) -> anyhow::Result<Self> {
        let default_table_size = section
            .default_table_size
            .as_deref()
            .map(TableSizeClass::parse)
            .transpose()?
            .unwrap_or(TableSizeClass::Medium);
        let config = Self {
            enabled: section.enabled.unwrap_or(true),
            warehouse: Warehouse::Generic,
            interval: section.interval.unwrap_or(0.80),
            default_runs_per_month: section.default_runs_per_month.unwrap_or(30.0),
            default_table_size,
            fail_on_monthly_delta: section.fail_on_monthly_delta,
            fail_on_monthly_delta_gb: section.fail_on_monthly_delta_gb,
            incremental_fraction: section.incremental_fraction.unwrap_or(0.05),
            pricing: section.pricing.unwrap_or(CostPricingSection {
                model: None,
                usd_per_tb: None,
                usd_per_credit: None,
                tb_per_credit_hour: None,
            }),
            inputs: section.inputs.unwrap_or(CostInputsSection {
                catalog: None,
                query_history: None,
                observations: None,
                observations_before: None,
                observations_after: None,
            }),
            sources: section.sources,
            models: section.models,
            rules: section
                .rules
                .into_iter()
                .map(|(k, v)| (k.to_ascii_uppercase(), v))
                .collect(),
        };
        config.validate()?;
        Ok(config)
    }

    pub fn dollar_mode(&self) -> bool {
        self.pricing.model.is_some()
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        validate_finite_range("cost.interval", self.interval, 0.5, 0.99)?;
        validate_positive("cost.default_runs_per_month", self.default_runs_per_month)?;
        validate_finite_range(
            "cost.incremental_fraction",
            self.incremental_fraction,
            0.001,
            1.0,
        )?;
        validate_optional_positive("cost.fail_on_monthly_delta", self.fail_on_monthly_delta)?;
        validate_optional_positive(
            "cost.fail_on_monthly_delta_gb",
            self.fail_on_monthly_delta_gb,
        )?;

        match self
            .pricing
            .model
            .as_deref()
            .map(|value| value.to_ascii_lowercase())
        {
            None => {
                if self.pricing.usd_per_tb.is_some()
                    || self.pricing.usd_per_credit.is_some()
                    || self.pricing.tb_per_credit_hour.is_some()
                {
                    anyhow::bail!("cost.pricing.model is required when pricing values are set");
                }
            }
            Some(model) if model == "scan" => {
                validate_optional_positive("cost.pricing.usd_per_tb", self.pricing.usd_per_tb)?;
            }
            Some(model) if model == "compute" => {
                validate_optional_positive(
                    "cost.pricing.usd_per_credit",
                    self.pricing.usd_per_credit,
                )?;
                if let Some(spec) = &self.pricing.tb_per_credit_hour {
                    validate_range_or_point("cost.pricing.tb_per_credit_hour", spec)?;
                }
            }
            Some(model) => anyhow::bail!("unknown cost pricing model '{model}'"),
        }

        for (name, source) in &self.sources {
            if let Some(bytes) = &source.bytes {
                let value = parse_bytes_spec(bytes)?;
                validate_positive(&format!("cost.sources.{name}.bytes"), value)?;
            }
            if let Some(rows) = &source.rows {
                validate_range_or_point(&format!("cost.sources.{name}.rows"), rows)?;
            }
            validate_optional_positive(
                &format!("cost.sources.{name}.avg_row_bytes"),
                source.avg_row_bytes,
            )?;
        }
        for (name, model) in &self.models {
            validate_optional_positive(
                &format!("cost.models.{name}.runs_per_month"),
                model.runs_per_month,
            )?;
        }
        for (rule, override_config) in &self.rules {
            if let Some(multiplier) = &override_config.multiplier {
                validate_range_or_point(&format!("cost.rules.{rule}.multiplier"), multiplier)?;
            }
        }
        Ok(())
    }
}

pub fn range_or_point_to_estimate(spec: &RangeOrPoint, default_cv: Option<f64>) -> crate::Estimate {
    match spec {
        RangeOrPoint::Point(value) => crate::Estimate::from_point(*value, default_cv),
        RangeOrPoint::Range { p10, p90 } => crate::Estimate::from_range(*p10, *p90),
    }
}

pub fn parse_bytes_spec(spec: &BytesSpec) -> anyhow::Result<f64> {
    let value = match spec {
        BytesSpec::Number(n) => Ok(*n),
        BytesSpec::Text(text) => parse_human_bytes(text),
    }?;
    validate_positive("bytes", value)?;
    Ok(value)
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
            let bytes = value * factor;
            validate_positive("bytes", bytes)?;
            return Ok(bytes);
        }
    }
    let bytes = trimmed
        .parse::<f64>()
        .map_err(|_| anyhow::anyhow!("invalid bytes '{text}'"))?;
    validate_positive("bytes", bytes)?;
    Ok(bytes)
}

fn validate_optional_positive(name: &str, value: Option<f64>) -> anyhow::Result<()> {
    if let Some(value) = value {
        validate_positive(name, value)?;
    }
    Ok(())
}

fn validate_positive(name: &str, value: f64) -> anyhow::Result<()> {
    if !value.is_finite() || value <= 0.0 {
        anyhow::bail!("{name} must be finite and greater than zero");
    }
    Ok(())
}

fn validate_finite_range(name: &str, value: f64, min: f64, max: f64) -> anyhow::Result<()> {
    if !value.is_finite() || value < min || value > max {
        anyhow::bail!("{name} must be finite and between {min} and {max}");
    }
    Ok(())
}

fn validate_range_or_point(name: &str, spec: &RangeOrPoint) -> anyhow::Result<()> {
    match spec {
        RangeOrPoint::Point(value) => validate_positive(name, *value),
        RangeOrPoint::Range { p10, p90 } => {
            validate_positive(&format!("{name}.p10"), *p10)?;
            validate_positive(&format!("{name}.p90"), *p90)?;
            if p90 <= p10 {
                anyhow::bail!("{name}.p90 must be greater than p10");
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_human_bytes() {
        assert_eq!(parse_human_bytes("12TB").unwrap(), 12_000_000_000_000.0);
        assert_eq!(parse_human_bytes("500 GB").unwrap(), 500_000_000_000.0);
    }

    #[test]
    fn rejects_invalid_cost_values_instead_of_clamping() {
        let section: CostSection = toml::from_str(
            r#"
default_runs_per_month = 0
"#,
        )
        .expect("parse section");
        assert!(CostConfig::from_section(section).is_err());

        let section: CostSection = toml::from_str(
            r#"
default_table_size = "enormous"
"#,
        )
        .expect("parse section");
        assert!(CostConfig::from_section(section).is_err());
    }
}
