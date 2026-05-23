use anyhow::{Context, Result};
use costguard_diagnostics::{Confidence, Severity};
use costguard_platform::Platform;
use costguard_rules::RuleOverrides;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::str::FromStr;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    #[default]
    Text,
    Json,
    Github,
    Markdown,
}

impl FromStr for OutputFormat {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "text" => Ok(Self::Text),
            "json" => Ok(Self::Json),
            "github" => Ok(Self::Github),
            "markdown" | "md" => Ok(Self::Markdown),
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
    pub base_branch: Option<String>,
    pub changed_only: bool,
    pub fail_on: Option<Severity>,
    pub min_confidence: Option<Confidence>,
    pub rule_overrides: RuleOverrides,
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
            base_branch: None,
            changed_only: false,
            fail_on: Some(Severity::High),
            min_confidence: None,
            rule_overrides: RuleOverrides::default(),
        }
    }
}

#[derive(Debug, Deserialize, Default)]
pub struct FileConfig {
    pub warehouse: Option<String>,
    pub dialect: Option<String>,
    pub scan: Option<ScanSection>,
    pub output: Option<OutputSection>,
    pub dbt: Option<DbtSection>,
    pub rules: Option<RuleOverrides>,
}

#[derive(Debug, Deserialize, Default)]
pub struct ScanSection {
    pub paths: Option<Vec<PathBuf>>,
    pub ignore: Option<Vec<PathBuf>>,
}

#[derive(Debug, Deserialize, Default)]
pub struct OutputSection {
    pub format: Option<String>,
    pub fail_on: Option<String>,
    pub min_confidence: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct DbtSection {
    pub manifest_path: Option<PathBuf>,
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
    }
    if let Some(dbt) = file_config.dbt {
        config.manifest_path = dbt.manifest_path;
    }
    if let Some(rules) = file_config.rules {
        config.rule_overrides = rules
            .into_iter()
            .map(|(key, value)| (key.to_ascii_uppercase(), value))
            .collect();
    }
    Ok(config)
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
    }
}
