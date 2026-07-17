use anyhow::{Context, Result};
use costguard_cost::{CostConfig, CostSection};
use costguard_diagnostics::{Confidence, Severity};
use costguard_rules::{RuleOverrides, RuleRegistry};
use costguard_sql::Platform;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::str::FromStr;

pub const DEFAULT_MAX_MANIFEST_BYTES: u64 = 512 * 1024 * 1024;
pub const DEFAULT_MAX_TOTAL_BASE_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// Strictness policy controlling parse-failure and metadata error tolerance.
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

/// Thresholds and flags that determine whether a scan is considered complete.
#[derive(Debug, Clone)]
pub struct AnalysisConfig {
    pub policy: AnalysisPolicy,
    pub require_manifest: bool,
    pub require_manifest_integrity: bool,
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
            require_manifest_integrity: false,
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
                require_manifest_integrity: self.require_manifest_integrity,
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

/// Output format for scan results and rule listings.
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

/// Full configuration for a scan or explain run.
#[derive(Debug, Clone)]
pub struct ScanConfig {
    pub root: PathBuf,
    pub paths: Vec<PathBuf>,
    pub platform: Platform,
    pub format: OutputFormat,
    pub manifest_path: Option<PathBuf>,
    pub base_manifest_path: Option<PathBuf>,
    pub max_manifest_bytes: u64,
    pub ignore: Vec<PathBuf>,
    pub max_file_bytes: Option<u64>,
    pub base_branch: Option<String>,
    pub changed_only: bool,
    pub fail_on: Option<Severity>,
    pub min_confidence: Option<Confidence>,
    pub min_confidence_filter: bool,
    pub rule_overrides: RuleOverrides,
    pub baseline_path: Option<PathBuf>,
    pub write_baseline_path: Option<PathBuf>,
    pub cost: Option<CostConfig>,
    pub analysis: AnalysisConfig,
    pub signed_policy: SignedPolicyConfig,
    pub owners: OwnersConfig,
    pub gate: Option<GateConfig>,
    pub waivers: Vec<Waiver>,
}

impl Default for ScanConfig {
    fn default() -> Self {
        Self {
            root: PathBuf::from("."),
            paths: Vec::new(),
            platform: Platform::Generic,
            format: OutputFormat::Text,
            manifest_path: None,
            base_manifest_path: None,
            max_manifest_bytes: DEFAULT_MAX_MANIFEST_BYTES,
            ignore: Vec::new(),
            max_file_bytes: None,
            base_branch: None,
            changed_only: false,
            fail_on: Some(Severity::High),
            min_confidence: None,
            min_confidence_filter: false,
            rule_overrides: RuleOverrides::default(),
            baseline_path: None,
            write_baseline_path: None,
            cost: None,
            analysis: AnalysisConfig::default(),
            signed_policy: SignedPolicyConfig::default(),
            owners: OwnersConfig::default(),
            gate: None,
            waivers: Vec::new(),
        }
    }
}

/// A scan configuration plus execution-only safety limits.
///
/// The wrapper keeps [`ScanConfig`] source-compatible while allowing project
/// configuration to carry limits that do not affect scan semantics.
#[derive(Debug, Clone)]
pub struct ResolvedScanRequest {
    pub config: ScanConfig,
    pub max_total_base_bytes: u64,
}

impl From<ScanConfig> for ResolvedScanRequest {
    fn from(config: ScanConfig) -> Self {
        Self {
            config,
            max_total_base_bytes: DEFAULT_MAX_TOTAL_BASE_BYTES,
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
    pub policy: Option<SignedPolicySection>,
    pub owners: Option<OwnersConfig>,
    pub gate: Option<GateConfig>,
    #[serde(default)]
    pub waivers: Vec<Waiver>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnersConfig {
    #[serde(default)]
    pub default: OwnerValue,
    #[serde(default)]
    pub codeowners: bool,
    #[serde(default)]
    pub paths: BTreeMap<String, OwnerValue>,
    #[serde(default)]
    pub tags: BTreeMap<String, OwnerValue>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(untagged)]
pub enum OwnerValue {
    #[default]
    Empty,
    One(String),
    Many(Vec<String>),
}

impl OwnerValue {
    pub fn values(&self) -> Vec<String> {
        match self {
            Self::Empty => Vec::new(),
            Self::One(value) => vec![value.clone()],
            Self::Many(values) => values.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GateMode {
    #[default]
    Block,
    Warn,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GateConfig {
    pub name: Option<String>,
    #[serde(default)]
    pub mode: GateMode,
    pub fail_on: Option<Severity>,
    pub min_confidence: Option<Confidence>,
    pub fail_on_monthly_delta: Option<f64>,
    pub fail_on_monthly_delta_gb: Option<f64>,
    pub fail_on_pr_cost_increase: Option<f64>,
    pub fail_on_blast_radius: Option<usize>,
    #[serde(default)]
    pub require_owner: bool,
    #[serde(default)]
    pub block_only_new: bool,
    #[serde(default)]
    pub scopes: Vec<GateScope>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GateScope {
    pub name: String,
    #[serde(default)]
    pub mode: GateMode,
    #[serde(default)]
    pub paths: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub owners: Vec<String>,
    pub fail_on: Option<Severity>,
    pub min_confidence: Option<Confidence>,
    pub fail_on_monthly_delta: Option<f64>,
    pub fail_on_monthly_delta_gb: Option<f64>,
    pub fail_on_blast_radius: Option<usize>,
    #[serde(default)]
    pub require_owner: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Waiver {
    pub id: String,
    pub finding_id: Option<String>,
    pub rule_id: Option<String>,
    pub path: String,
    pub owner: String,
    pub reason: String,
    pub ticket_url: String,
    pub approver: String,
    pub created_at: String,
    pub expires_at: String,
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
    pub min_confidence_filter: Option<bool>,
    pub baseline: Option<PathBuf>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct DbtSection {
    pub manifest_path: Option<PathBuf>,
    pub base_manifest_path: Option<PathBuf>,
    pub max_manifest_bytes: Option<u64>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct AnalysisSection {
    pub policy: Option<String>,
    pub require_manifest: Option<bool>,
    pub require_manifest_integrity: Option<bool>,
    pub max_parse_failure_rate: Option<f64>,
    pub max_compiled_parse_failures: Option<usize>,
    pub max_skipped_files: Option<usize>,
    pub fail_on_metadata_errors: Option<bool>,
}

#[derive(Debug, Clone, Default)]
pub struct SignedPolicyConfig {
    pub bundle_path: Option<PathBuf>,
    pub trust_store_path: Option<PathBuf>,
    pub organization: Option<String>,
    pub team: Option<String>,
    pub repository: Option<String>,
    pub required: bool,
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct SignedPolicySection {
    pub bundle: Option<PathBuf>,
    pub trust_store: Option<PathBuf>,
    pub organization: Option<String>,
    pub team: Option<String>,
    pub repository: Option<String>,
    pub required: Option<bool>,
}

pub fn load_config(root: &Path) -> Result<FileConfig> {
    Ok(load_config_document(root)?.0)
}

/// Load and apply project configuration, including execution-only limits.
pub fn load_resolved_config(root: &Path, config: ScanConfig) -> Result<ResolvedScanRequest> {
    let (file_config, max_total_base_bytes) = load_config_document(root)?;
    Ok(ResolvedScanRequest {
        config: apply_file_config(config, file_config)?,
        max_total_base_bytes,
    })
}

fn load_config_document(root: &Path) -> Result<(FileConfig, u64)> {
    let path = root.join("costguard.toml");
    if !path.exists() {
        return Ok((FileConfig::default(), DEFAULT_MAX_TOTAL_BASE_BYTES));
    }
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let mut document: toml::Value =
        toml::from_str(&text).with_context(|| format!("invalid config {}", path.display()))?;
    let configured_limit = document
        .get_mut("scan")
        .and_then(toml::Value::as_table_mut)
        .and_then(|scan| scan.remove("max_total_base_bytes"));
    let max_total_base_bytes = match configured_limit {
        None | Some(toml::Value::Integer(0)) => DEFAULT_MAX_TOTAL_BASE_BYTES,
        Some(toml::Value::Integer(value)) if value > 0 => value as u64,
        Some(_) => anyhow::bail!(
            "invalid config {}: scan.max_total_base_bytes must be a non-negative integer",
            path.display()
        ),
    };
    let file_config = document
        .try_into()
        .with_context(|| format!("invalid config {}", path.display()))?;
    Ok((file_config, max_total_base_bytes))
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
        if let Some(min_confidence_filter) = output.min_confidence_filter {
            config.min_confidence_filter = min_confidence_filter;
        }
        if let Some(baseline) = output.baseline {
            config.baseline_path = Some(baseline);
        }
    }
    if let Some(dbt) = file_config.dbt {
        if let Some(path) = dbt.manifest_path {
            config.manifest_path = Some(path);
        }
        if let Some(path) = dbt.base_manifest_path {
            config.base_manifest_path = Some(path);
        }
        if let Some(max_manifest_bytes) = dbt.max_manifest_bytes {
            config.max_manifest_bytes = if max_manifest_bytes == 0 {
                DEFAULT_MAX_MANIFEST_BYTES
            } else {
                max_manifest_bytes
            };
        }
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
    if let Some(cost) = config.cost.as_mut() {
        cost.warehouse = costguard_cost::Warehouse::parse(&config.platform.to_string());
    }
    if let Some(analysis) = file_config.analysis {
        if let Some(policy) = analysis.policy {
            config.analysis.policy = policy.parse().map_err(anyhow::Error::msg)?;
        }
        if let Some(value) = analysis.require_manifest {
            config.analysis.require_manifest = value;
        }
        if let Some(value) = analysis.require_manifest_integrity {
            config.analysis.require_manifest_integrity = value;
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
    if let Some(policy) = file_config.policy {
        config.signed_policy.bundle_path = policy.bundle;
        config.signed_policy.trust_store_path = policy.trust_store;
        config.signed_policy.organization = policy.organization;
        config.signed_policy.team = policy.team;
        config.signed_policy.repository = policy.repository;
        config.signed_policy.required = policy.required.unwrap_or(false);
    }
    if let Some(owners) = file_config.owners {
        config.owners = owners;
    }
    config.gate = file_config.gate;
    config.waivers = file_config.waivers;
    Ok(config)
}

#[derive(Debug, Clone, Default)]
pub struct ScanRuntimeOverrides {
    pub warehouse: Option<String>,
    pub dialect: Option<String>,
    pub format: Option<OutputFormat>,
    pub manifest_path: Option<PathBuf>,
    pub base_manifest_path: Option<PathBuf>,
    pub fail_on: Option<String>,
    pub min_confidence: Option<String>,
    pub min_confidence_filter: Option<bool>,
    pub baseline_path: Option<PathBuf>,
    pub write_baseline_path: Option<PathBuf>,
    pub cost: bool,
    pub fail_on_cost_delta: Option<f64>,
    pub min_cost_coverage: Option<f64>,
    pub block_only_new: Option<bool>,
    pub fail_on_pr_cost_increase: Option<f64>,
    pub analysis_policy: Option<String>,
    pub policy_bundle_path: Option<PathBuf>,
    pub trust_store_path: Option<PathBuf>,
    pub policy_organization: Option<String>,
    pub policy_team: Option<String>,
    pub policy_repository: Option<String>,
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
        if let Some(base_manifest) = &self.base_manifest_path {
            config.base_manifest_path = Some(base_manifest.clone());
        }
        if let Some(fail_on) = &self.fail_on {
            config.fail_on = Some(fail_on.parse::<Severity>().map_err(anyhow::Error::msg)?);
        }
        if let Some(min_confidence) = &self.min_confidence {
            config.min_confidence = Some(min_confidence.parse().map_err(anyhow::Error::msg)?);
        }
        if let Some(min_confidence_filter) = self.min_confidence_filter {
            config.min_confidence_filter = min_confidence_filter;
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
        if let Some(fraction) = self.min_cost_coverage {
            let mut cost_config = config.cost.take().unwrap_or_else(|| CostConfig {
                enabled: true,
                ..CostConfig::default()
            });
            cost_config.enabled = true;
            cost_config.min_mapped_spend_fraction = Some(fraction);
            config.cost = Some(cost_config);
        }
        if self.block_only_new.is_some() || self.fail_on_pr_cost_increase.is_some() {
            let gate = config.gate.get_or_insert_with(GateConfig::default);
            if let Some(block_only_new) = self.block_only_new {
                gate.block_only_new = block_only_new;
            }
            if let Some(threshold) = self.fail_on_pr_cost_increase {
                gate.fail_on_pr_cost_increase = Some(threshold);
                let mut cost = config.cost.take().unwrap_or_default();
                cost.enabled = true;
                config.cost = Some(cost);
            }
        }
        if let Some(policy) = &self.analysis_policy {
            config.analysis.policy = policy.parse().map_err(anyhow::Error::msg)?;
        }
        if let Some(path) = &self.policy_bundle_path {
            config.signed_policy.bundle_path = Some(path.clone());
        }
        if let Some(path) = &self.trust_store_path {
            config.signed_policy.trust_store_path = Some(path.clone());
        }
        if let Some(value) = &self.policy_organization {
            config.signed_policy.organization = Some(value.clone());
        }
        if let Some(value) = &self.policy_team {
            config.signed_policy.team = Some(value.clone());
        }
        if let Some(value) = &self.policy_repository {
            config.signed_policy.repository = Some(value.clone());
        }
        if let Some(cost) = config.cost.as_mut() {
            cost.warehouse = costguard_cost::Warehouse::parse(&config.platform.to_string());
        }
        Ok(())
    }
}

pub fn validate_scan_config(config: &ScanConfig) -> Result<()> {
    config.analysis.validate()?;
    if let Some(path) = &config.base_manifest_path {
        let resolved = if path.is_absolute() {
            path.clone()
        } else {
            config.root.join(path)
        };
        if !resolved.exists() {
            anyhow::bail!("base manifest path does not exist: {}", resolved.display());
        }
    }
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
    validate_owners(&config.owners)?;
    if let Some(gate) = &config.gate {
        validate_gate(gate)?;
        if gate.fail_on_pr_cost_increase.is_some()
            && !config
                .cost
                .as_ref()
                .is_some_and(|cost| cost.enabled && cost.dollar_mode())
        {
            anyhow::bail!(
                "gate fail_on_pr_cost_increase requires enabled [cost] and [cost.pricing].model = 'scan' or 'compute'"
            );
        }
    }
    validate_waivers(&config.waivers)?;
    if config.signed_policy.required && config.signed_policy.bundle_path.is_none() {
        anyhow::bail!("signed policy is required but no bundle is configured");
    }
    if config.signed_policy.bundle_path.is_some() && config.signed_policy.trust_store_path.is_none()
    {
        anyhow::bail!("policy.trust_store is required when policy.bundle is configured");
    }
    for (label, path) in [
        ("policy bundle", config.signed_policy.bundle_path.as_ref()),
        (
            "policy trust store",
            config.signed_policy.trust_store_path.as_ref(),
        ),
    ] {
        if let Some(path) = path {
            let resolved = if path.is_absolute() {
                path.clone()
            } else {
                config.root.join(path)
            };
            if !resolved.is_file() {
                anyhow::bail!("{label} does not exist: {}", resolved.display());
            }
        }
    }
    Ok(())
}

fn validate_owners(config: &OwnersConfig) -> Result<()> {
    for owner in config
        .default
        .values()
        .into_iter()
        .chain(config.paths.values().flat_map(OwnerValue::values))
        .chain(config.tags.values().flat_map(OwnerValue::values))
    {
        if owner.trim().is_empty() {
            anyhow::bail!("owners entries cannot be empty");
        }
    }
    for pattern in config.paths.keys() {
        globset::Glob::new(pattern)
            .with_context(|| format!("invalid owners.paths glob '{pattern}'"))?;
    }
    Ok(())
}

fn validate_gate(gate: &GateConfig) -> Result<()> {
    validate_gate_values(
        gate.name.as_deref().unwrap_or("default"),
        gate.fail_on_monthly_delta,
        gate.fail_on_monthly_delta_gb,
        gate.fail_on_blast_radius,
    )?;
    if gate
        .fail_on_pr_cost_increase
        .is_some_and(|value| !value.is_finite() || value <= 0.0)
    {
        anyhow::bail!(
            "gate '{}' fail_on_pr_cost_increase must be finite and positive",
            gate.name.as_deref().unwrap_or("default")
        );
    }
    let mut names = HashSet::new();
    for scope in &gate.scopes {
        if !names.insert(scope.name.as_str()) {
            anyhow::bail!("duplicate gate scope name '{}'", scope.name);
        }
        if scope.paths.is_empty() && scope.tags.is_empty() && scope.owners.is_empty() {
            anyhow::bail!(
                "gate scope '{}' must specify paths, tags, or owners",
                scope.name
            );
        }
        for pattern in &scope.paths {
            globset::Glob::new(pattern).with_context(|| {
                format!("invalid gate scope '{}' path glob '{pattern}'", scope.name)
            })?;
        }
        validate_gate_values(
            &scope.name,
            scope.fail_on_monthly_delta,
            scope.fail_on_monthly_delta_gb,
            scope.fail_on_blast_radius,
        )?;
    }
    Ok(())
}

fn validate_gate_values(
    name: &str,
    monthly: Option<f64>,
    monthly_gb: Option<f64>,
    blast_radius: Option<usize>,
) -> Result<()> {
    if name.trim().is_empty() {
        anyhow::bail!("gate name cannot be empty");
    }
    for (field, value) in [
        ("fail_on_monthly_delta", monthly),
        ("fail_on_monthly_delta_gb", monthly_gb),
    ] {
        if value.is_some_and(|value| !value.is_finite() || value <= 0.0) {
            anyhow::bail!("gate '{name}' {field} must be finite and positive");
        }
    }
    if blast_radius == Some(0) {
        anyhow::bail!("gate '{name}' fail_on_blast_radius must be positive");
    }
    Ok(())
}

fn validate_waivers(waivers: &[Waiver]) -> Result<()> {
    let mut ids = HashSet::new();
    for waiver in waivers {
        if !ids.insert(waiver.id.as_str()) {
            anyhow::bail!("duplicate waiver id '{}'", waiver.id);
        }
        for (field, value) in [
            ("id", waiver.id.as_str()),
            ("path", waiver.path.as_str()),
            ("owner", waiver.owner.as_str()),
            ("reason", waiver.reason.as_str()),
            ("ticket_url", waiver.ticket_url.as_str()),
            ("approver", waiver.approver.as_str()),
        ] {
            if value.trim().is_empty() {
                anyhow::bail!("waiver '{}' {field} cannot be empty", waiver.id);
            }
        }
        if waiver.finding_id.is_some() == waiver.rule_id.is_some() {
            anyhow::bail!(
                "waiver '{}' must specify exactly one of finding_id or rule_id",
                waiver.id
            );
        }
        globset::Glob::new(&waiver.path)
            .with_context(|| format!("invalid waiver '{}' path glob", waiver.id))?;
        let created = chrono::DateTime::parse_from_rfc3339(&waiver.created_at)
            .with_context(|| format!("waiver '{}' created_at must be RFC3339", waiver.id))?;
        let expires = chrono::DateTime::parse_from_rfc3339(&waiver.expires_at)
            .with_context(|| format!("waiver '{}' expires_at must be RFC3339", waiver.id))?;
        if expires <= created {
            anyhow::bail!("waiver '{}' expires_at must be after created_at", waiver.id);
        }
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
    fn resolved_config_loads_aggregate_base_limit_without_changing_scan_config() {
        let root = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            root.path().join("costguard.toml"),
            "[scan]\nmax_total_base_bytes = 1234\n",
        )
        .expect("write config");
        let request = load_resolved_config(
            root.path(),
            ScanConfig {
                root: root.path().to_path_buf(),
                ..ScanConfig::default()
            },
        )
        .expect("load resolved config");
        assert_eq!(request.max_total_base_bytes, 1234);

        std::fs::write(
            root.path().join("costguard.toml"),
            "[scan]\nmax_total_base_bytes = 0\n",
        )
        .expect("write config");
        assert_eq!(
            load_resolved_config(root.path(), ScanConfig::default())
                .expect("load default limit")
                .max_total_base_bytes,
            DEFAULT_MAX_TOTAL_BASE_BYTES
        );
    }

    #[test]
    fn resolved_config_still_rejects_unknown_scan_keys() {
        let root = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            root.path().join("costguard.toml"),
            "[scan]\nmax_total_base_bytes = 1234\nunknown = true\n",
        )
        .expect("write config");
        let error = format!(
            "{:#}",
            load_resolved_config(root.path(), ScanConfig::default()).expect_err("unknown key")
        );
        assert!(error.contains("unknown field"), "{error}");
    }

    #[test]
    fn manifest_limit_applies_and_zero_restores_default() {
        let configured = apply_file_config(
            ScanConfig::default(),
            toml::from_str("[dbt]\nmax_manifest_bytes = 1024\n").expect("parse config"),
        )
        .expect("apply config");
        assert_eq!(configured.max_manifest_bytes, 1024);

        let defaulted = apply_file_config(
            configured,
            toml::from_str("[dbt]\nmax_manifest_bytes = 0\n").expect("parse config"),
        )
        .expect("apply config");
        assert_eq!(defaulted.max_manifest_bytes, DEFAULT_MAX_MANIFEST_BYTES);
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

    #[test]
    fn parses_and_validates_owners_gates_and_waivers() {
        let file: FileConfig = toml::from_str(
            r#"
[owners]
default = "@data"
codeowners = true

[owners.paths]
"models/finance/**" = ["@finance"]

[owners.tags]
critical = "@platform"

[gate]
name = "default"
fail_on = "high"
min_confidence = "medium"
require_owner = true

[[gate.scopes]]
name = "finance"
mode = "warn"
paths = ["models/finance/**"]
owners = ["@finance"]
fail_on_monthly_delta = 100.0
fail_on_blast_radius = 2

[[waivers]]
id = "CG-1"
rule_id = "SQLCOST001"
path = "models/finance/**"
owner = "@finance"
reason = "migration"
ticket_url = "https://example.com/CG-1"
approver = "@lead"
created_at = "2026-01-01T00:00:00Z"
expires_at = "2026-12-01T00:00:00Z"
"#,
        )
        .unwrap();
        let config = apply_file_config(ScanConfig::default(), file).unwrap();
        validate_scan_config(&config).unwrap();
        assert_eq!(config.owners.default.values(), vec!["@data"]);
        assert_eq!(config.gate.unwrap().scopes.len(), 1);
        assert_eq!(config.waivers.len(), 1);
    }

    #[test]
    fn rejects_unscoped_gate_scope() {
        let file: FileConfig = toml::from_str(
            r#"
[gate]
[[gate.scopes]]
name = "bad"

[[waivers]]
id = "CG-1"
finding_id = "finding"
rule_id = "SQLCOST001"
path = "models/**"
owner = "@data"
reason = "migration"
ticket_url = "https://example.com/CG-1"
approver = "@lead"
created_at = "2026-01-01T00:00:00Z"
expires_at = "2026-12-01T00:00:00Z"
"#,
        )
        .unwrap();
        let config = apply_file_config(ScanConfig::default(), file).unwrap();
        let error = validate_scan_config(&config).unwrap_err().to_string();
        assert!(
            error.contains("must specify paths, tags, or owners"),
            "{error}"
        );
    }

    #[test]
    fn rejects_ambiguous_waiver_matcher() {
        let file: FileConfig = toml::from_str(
            r#"
[[waivers]]
id = "CG-1"
finding_id = "finding"
rule_id = "SQLCOST001"
path = "models/**"
owner = "@data"
reason = "migration"
ticket_url = "https://example.com/CG-1"
approver = "@lead"
created_at = "2026-01-01T00:00:00Z"
expires_at = "2026-12-01T00:00:00Z"
"#,
        )
        .unwrap();
        let config = apply_file_config(ScanConfig::default(), file).unwrap();
        let error = validate_scan_config(&config).unwrap_err().to_string();
        assert!(
            error.contains("exactly one of finding_id or rule_id"),
            "{error}"
        );
    }

    #[test]
    fn pr_cost_gate_requires_positive_threshold_and_dollar_pricing() {
        let parse = |source: &str| {
            apply_file_config(
                ScanConfig::default(),
                toml::from_str(source).expect("parse config"),
            )
            .expect("apply config")
        };
        let missing_pricing = parse(
            r#"
[gate]
fail_on_pr_cost_increase = 100

[cost]
enabled = true
"#,
        );
        let error = validate_scan_config(&missing_pricing)
            .unwrap_err()
            .to_string();
        assert!(error.contains("[cost.pricing].model"), "{error}");

        let invalid = parse(
            r#"
[gate]
fail_on_pr_cost_increase = 0
"#,
        );
        let error = validate_scan_config(&invalid).unwrap_err().to_string();
        assert!(error.contains("finite and positive"), "{error}");
        for threshold in [f64::NAN, f64::INFINITY, -1.0] {
            let gate = GateConfig {
                fail_on_pr_cost_increase: Some(threshold),
                ..GateConfig::default()
            };
            assert!(validate_gate(&gate).is_err(), "accepted {threshold}");
        }

        let valid = parse(
            r#"
[gate]
fail_on_pr_cost_increase = 100

[cost]
enabled = true

[cost.pricing]
model = "scan"
usd_per_tb = 6.25
"#,
        );
        validate_scan_config(&valid).expect("valid priced PR cost gate");
    }

    #[test]
    fn runtime_pr_overrides_enable_cost_and_take_precedence() {
        let mut config = ScanConfig {
            gate: Some(GateConfig {
                block_only_new: false,
                fail_on_pr_cost_increase: Some(10.0),
                ..GateConfig::default()
            }),
            ..ScanConfig::default()
        };
        ScanRuntimeOverrides {
            block_only_new: Some(true),
            fail_on_pr_cost_increase: Some(250.0),
            ..ScanRuntimeOverrides::default()
        }
        .apply_to(&mut config)
        .expect("apply overrides");
        let gate = config.gate.expect("gate");
        assert!(gate.block_only_new);
        assert_eq!(gate.fail_on_pr_cost_increase, Some(250.0));
        assert!(config.cost.expect("cost").enabled);
    }
}
