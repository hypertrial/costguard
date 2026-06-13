use anyhow::{Context, Result};
use costguard_cost::{CostConfig, CostSection};
use costguard_diagnostics::{Confidence, Severity};
use costguard_platform::Platform;
use costguard_rules::{RuleOverrides, RuleRegistry};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::str::FromStr;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AnalysisPolicy {
    #[default]
    Standard,
    Strict,
}

impl FromStr for AnalysisPolicy {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "standard" => Ok(Self::Standard),
            "strict" => Ok(Self::Strict),
            other => Err(format!("unknown analysis policy '{other}'")),
        }
    }
}

#[derive(Debug, Clone)]
pub struct AnalysisConfig {
    pub policy: AnalysisPolicy,
    pub require_manifest: bool,
    pub max_parse_failure_rate: f64,
    pub max_compiled_parse_failures: usize,
    pub max_skipped_files: usize,
    pub fail_on_metadata_errors: bool,
}

impl Default for AnalysisConfig {
    fn default() -> Self {
        Self {
            policy: AnalysisPolicy::Standard,
            require_manifest: false,
            max_parse_failure_rate: 1.0,
            max_compiled_parse_failures: usize::MAX,
            max_skipped_files: usize::MAX,
            fail_on_metadata_errors: false,
        }
    }
}

impl AnalysisConfig {
    pub fn effective(&self) -> Self {
        if self.policy == AnalysisPolicy::Strict {
            Self {
                policy: AnalysisPolicy::Strict,
                require_manifest: true,
                max_parse_failure_rate: 0.0,
                max_compiled_parse_failures: 0,
                max_skipped_files: 0,
                fail_on_metadata_errors: true,
            }
        } else {
            self.clone()
        }
    }

    pub fn validate(&self) -> Result<()> {
        if !self.max_parse_failure_rate.is_finite()
            || !(0.0..=1.0).contains(&self.max_parse_failure_rate)
        {
            anyhow::bail!("analysis.max_parse_failure_rate must be finite and between 0 and 1");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    #[default]
    Text,
    Json,
    Github,
    Markdown,
    Sarif,
}

impl FromStr for OutputFormat {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "text" => Ok(Self::Text),
            "json" => Ok(Self::Json),
            "github" => Ok(Self::Github),
            "markdown" | "md" => Ok(Self::Markdown),
            "sarif" => Ok(Self::Sarif),
            other => Err(format!("unknown output format '{other}'")),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ScanConfig {
    pub root: PathBuf,
    pub paths: Vec<PathBuf>,
    pub platform: Platform,
    pub format: OutputFormat,
    pub manifest_path: Option<PathBuf>,
    pub ignore: Vec<PathBuf>,
    pub max_file_bytes: Option<u64>,
    pub base_branch: Option<String>,
    pub changed_only: bool,
    pub fail_on: Option<Severity>,
    pub min_confidence: Option<Confidence>,
    pub rule_overrides: RuleOverrides,
    pub baseline_path: Option<PathBuf>,
    pub write_baseline_path: Option<PathBuf>,
    pub cost: Option<CostConfig>,
    pub analysis: AnalysisConfig,
}

impl Default for ScanConfig {
    fn default() -> Self {
        Self {
            root: PathBuf::from("."),
            paths: Vec::new(),
            platform: Platform::Generic,
            format: OutputFormat::Text,
            manifest_path: None,
            ignore: Vec::new(),
            max_file_bytes: None,
            base_branch: None,
            changed_only: false,
            fail_on: Some(Severity::High),
            min_confidence: None,
            rule_overrides: RuleOverrides::default(),
            baseline_path: None,
            write_baseline_path: None,
            cost: None,
            analysis: AnalysisConfig::default(),
        }
    }
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct FileConfig {
    pub warehouse: Option<String>,
    pub dialect: Option<String>,
    pub scan: Option<ScanSection>,
    pub output: Option<OutputSection>,
    pub dbt: Option<DbtSection>,
    pub rules: Option<RuleOverrides>,
    pub cost: Option<CostSection>,
    pub analysis: Option<AnalysisSection>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ScanSection {
    pub paths: Option<Vec<PathBuf>>,
    pub ignore: Option<Vec<PathBuf>>,
    pub max_file_bytes: Option<u64>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct OutputSection {
    pub format: Option<String>,
    pub fail_on: Option<String>,
    pub min_confidence: Option<String>,
    pub baseline: Option<PathBuf>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct DbtSection {
    pub manifest_path: Option<PathBuf>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct AnalysisSection {
    pub policy: Option<String>,
    pub require_manifest: Option<bool>,
    pub max_parse_failure_rate: Option<f64>,
    pub max_compiled_parse_failures: Option<usize>,
    pub max_skipped_files: Option<usize>,
    pub fail_on_metadata_errors: Option<bool>,
}

pub fn load_config(root: &Path) -> Result<FileConfig> {
    let path = root.join("costguard.toml");
    if !path.exists() {
        return Ok(FileConfig::default());
    }
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    toml::from_str(&text).with_context(|| format!("invalid config {}", path.display()))
}

pub fn apply_file_config(mut config: ScanConfig, file_config: FileConfig) -> Result<ScanConfig> {
    if let Some(warehouse) = file_config.warehouse {
        config.platform = warehouse.parse().map_err(anyhow::Error::msg)?;
    } else if let Some(dialect) = file_config.dialect {
        config.platform = dialect.parse().map_err(anyhow::Error::msg)?;
    }
    if let Some(scan) = file_config.scan {
        if let Some(paths) = scan.paths {
            config.paths = paths;
        }
        if let Some(ignore) = scan.ignore {
            config.ignore = ignore;
        }
        if let Some(max_file_bytes) = scan.max_file_bytes {
            config.max_file_bytes = Some(max_file_bytes);
        }
    }
    if let Some(output) = file_config.output {
        if let Some(format) = output.format {
            config.format = format.parse().map_err(anyhow::Error::msg)?;
        }
        if let Some(fail_on) = output.fail_on {
            config.fail_on = Some(fail_on.parse().map_err(anyhow::Error::msg)?);
        }
        if let Some(min_confidence) = output.min_confidence {
            config.min_confidence = Some(min_confidence.parse().map_err(anyhow::Error::msg)?);
        }
        if let Some(baseline) = output.baseline {
            config.baseline_path = Some(baseline);
        }
    }
    if let Some(dbt) = file_config.dbt {
        config.manifest_path = dbt.manifest_path;
    }
    if let Some(rules) = file_config.rules {
        let known = RuleRegistry::default_rules()
            .metadata()
            .into_iter()
            .map(|rule| rule.id)
            .collect::<std::collections::HashSet<_>>();
        let mut overrides = RuleOverrides::default();
        for (key, value) in rules {
            let normalized = key.to_ascii_uppercase();
            if !known.contains(normalized.as_str()) {
                anyhow::bail!("unknown rule id '{key}'");
            }
            overrides.insert(normalized, value);
        }
        config.rule_overrides = overrides;
    }
    if let Some(cost) = file_config.cost {
        config.cost = Some(CostConfig::from_section(cost)?);
    }
    if let Some(analysis) = file_config.analysis {
        if let Some(policy) = analysis.policy {
            config.analysis.policy = policy.parse().map_err(anyhow::Error::msg)?;
        }
        if let Some(value) = analysis.require_manifest {
            config.analysis.require_manifest = value;
        }
        if let Some(value) = analysis.max_parse_failure_rate {
            config.analysis.max_parse_failure_rate = value;
        }
        if let Some(value) = analysis.max_compiled_parse_failures {
            config.analysis.max_compiled_parse_failures = value;
        }
        if let Some(value) = analysis.max_skipped_files {
            config.analysis.max_skipped_files = value;
        }
        if let Some(value) = analysis.fail_on_metadata_errors {
            config.analysis.fail_on_metadata_errors = value;
        }
    }
    Ok(config)
}

#[derive(Debug, Clone, Default)]
pub struct ScanRuntimeOverrides {
    pub warehouse: Option<String>,
    pub dialect: Option<String>,
    pub format: Option<OutputFormat>,
    pub manifest_path: Option<PathBuf>,
    pub fail_on: Option<String>,
    pub min_confidence: Option<String>,
    pub baseline_path: Option<PathBuf>,
    pub write_baseline_path: Option<PathBuf>,
    pub cost: bool,
    pub fail_on_cost_delta: Option<f64>,
    pub analysis_policy: Option<String>,
}

impl ScanRuntimeOverrides {
    pub fn apply_to(&self, config: &mut ScanConfig) -> Result<()> {
        if let Some(warehouse) = &self.warehouse {
            config.platform = warehouse.parse::<Platform>().map_err(anyhow::Error::msg)?;
        } else if let Some(dialect) = &self.dialect {
            config.platform = dialect.parse::<Platform>().map_err(anyhow::Error::msg)?;
        }
        if let Some(format) = &self.format {
            config.format = *format;
        }
        if let Some(manifest) = &self.manifest_path {
            config.manifest_path = Some(manifest.clone());
        }
        if let Some(fail_on) = &self.fail_on {
            config.fail_on = Some(fail_on.parse::<Severity>().map_err(anyhow::Error::msg)?);
        }
        if let Some(min_confidence) = &self.min_confidence {
            config.min_confidence = Some(min_confidence.parse().map_err(anyhow::Error::msg)?);
        }
        if let Some(baseline) = &self.baseline_path {
            config.baseline_path = Some(baseline.clone());
        }
        if let Some(write_baseline) = &self.write_baseline_path {
            config.write_baseline_path = Some(write_baseline.clone());
        }
        if self.cost {
            let mut cost_config = config.cost.take().unwrap_or_default();
            cost_config.enabled = true;
            config.cost = Some(cost_config);
        }
        if let Some(delta) = self.fail_on_cost_delta {
            let mut cost_config = config.cost.take().unwrap_or_else(|| CostConfig {
                enabled: true,
                ..CostConfig::default()
            });
            cost_config.enabled = true;
            cost_config.fail_on_monthly_delta = Some(delta);
            config.cost = Some(cost_config);
        }
        if let Some(policy) = &self.analysis_policy {
            config.analysis.policy = policy.parse().map_err(anyhow::Error::msg)?;
        }
        Ok(())
    }
}

pub fn validate_scan_config(config: &ScanConfig) -> Result<()> {
    config.analysis.validate()?;
    if let Some(path) = &config.manifest_path {
        let resolved = if path.is_absolute() {
            path.clone()
        } else {
            config.root.join(path)
        };
        if !resolved.exists() {
            anyhow::bail!("manifest path does not exist: {}", resolved.display());
        }
    }
    if let Some(path) = &config.baseline_path {
        let resolved = if path.is_absolute() {
            path.clone()
        } else {
            config.root.join(path)
        };
        if !resolved.exists() {
            anyhow::bail!("baseline path does not exist: {}", resolved.display());
        }
        let baseline = crate::baseline::load_finding_baseline(&resolved)?;
        crate::baseline::validate_finding_baseline(&baseline, config.platform)?;
    }
    if let Some(cost) = &config.cost {
        cost.validate()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_format_parses() {
        assert_eq!("json".parse::<OutputFormat>().unwrap(), OutputFormat::Json);
        assert_eq!(
            "github".parse::<OutputFormat>().unwrap(),
            OutputFormat::Github
        );
        assert_eq!(
            "markdown".parse::<OutputFormat>().unwrap(),
            OutputFormat::Markdown
        );
        assert_eq!(
            "sarif".parse::<OutputFormat>().unwrap(),
            OutputFormat::Sarif
        );
    }

    #[test]
    fn max_file_bytes_applies_from_file_config() {
        let config = apply_file_config(
            ScanConfig::default(),
            toml::from_str(
                r#"
[scan]
max_file_bytes = 1024
"#,
            )
            .expect("parse config"),
        )
        .expect("apply config");
        assert_eq!(config.max_file_bytes, Some(1024));
    }

    #[test]
    fn strict_analysis_policy_is_fail_closed() {
        let config = apply_file_config(
            ScanConfig::default(),
            toml::from_str(
                r#"
[analysis]
policy = "strict"
max_parse_failure_rate = 0.5
max_skipped_files = 10
"#,
            )
            .expect("parse config"),
        )
        .expect("apply config");
        let effective = config.analysis.effective();
        assert!(effective.require_manifest);
        assert_eq!(effective.max_parse_failure_rate, 0.0);
        assert_eq!(effective.max_skipped_files, 0);
        assert!(effective.fail_on_metadata_errors);
    }
}
