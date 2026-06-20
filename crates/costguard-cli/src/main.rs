//! Costguard CLI binary.
//!
//! Entry point for `scan`, `explain`, `pr`, `cost`, `rules`, and
//! `policy` subcommands. Delegates scan orchestration to
//! [`costguard_core`].

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use costguard_core::{
    apply_file_config, explain, init_project, load_config, rules, scan, validate_scan_config,
    InitOptions, OutputFormat, PrSummary, ReceiptTrend, ScanConfig, ScanRuntimeOverrides,
};
use costguard_cost::{normalize_cost_export, CostExportFormat, NormalizeCostOptions};
use costguard_output::{render, render_rules};
use costguard_policy::{
    canonical_json, compile_toml, generate_key, policy_digest, read_signed_policy,
    read_trust_store, resolve_policy, sign_policy, verify_policy, PolicyDocumentV2,
    ResolutionContext, TrustStoreV1, TrustedKeyV1,
};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Debug, Parser)]
#[command(name = "costguard")]
#[command(about = "Local, dbt-aware cost regression guardrail for git workflows")]
#[command(version, propagate_version = true)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Scan(ScanArgs),
    Explain(ExplainArgs),
    Pr(PrArgs),
    Cost(CostArgs),
    Rules(RulesArgs),
    Policy(PolicyCommandArgs),
    Init(InitArgs),
}

#[derive(Debug, Parser)]
struct PolicyCommandArgs {
    #[command(subcommand)]
    command: PolicyCommand,
}

#[derive(Debug, Subcommand)]
enum PolicyCommand {
    Keygen {
        key_id: String,
        #[arg(long)]
        private_key: PathBuf,
        #[arg(long)]
        trust_store: PathBuf,
        #[arg(long)]
        valid_until: Option<String>,
    },
    Compile {
        input: PathBuf,
        output: PathBuf,
    },
    Sign {
        policy: PathBuf,
        key: PathBuf,
        output: PathBuf,
    },
    Verify {
        bundle: PathBuf,
        trust_store: PathBuf,
    },
    Resolve {
        bundle: PathBuf,
        trust_store: PathBuf,
        #[arg(long)]
        organization: String,
        #[arg(long)]
        repository: String,
        #[arg(long)]
        team: Option<String>,
        #[arg(long)]
        path: Option<String>,
    },
    Inspect {
        bundle: PathBuf,
        trust_store: PathBuf,
    },
}

#[derive(Debug, Parser)]
struct ScanArgs {
    paths: Vec<PathBuf>,
    #[command(flatten)]
    common: CommonScanArgs,
    #[arg(long)]
    fail_on: Option<String>,
    #[arg(long)]
    min_confidence: Option<String>,
    #[arg(long)]
    min_confidence_filter: bool,
    #[arg(long)]
    baseline: Option<PathBuf>,
    #[arg(long)]
    write_baseline: Option<PathBuf>,
    #[arg(long)]
    cost: bool,
    #[arg(long)]
    fail_on_cost_delta: Option<f64>,
    #[arg(long, value_name = "0.0..1.0")]
    min_cost_coverage: Option<f64>,
    #[command(flatten)]
    receipt: ReceiptArgs,
}

#[derive(Debug, Parser)]
struct CostArgs {
    #[command(subcommand)]
    command: CostCommand,
}

#[derive(Debug, Subcommand)]
enum CostCommand {
    Report(CostReportArgs),
    Normalize {
        input: PathBuf,
        output: PathBuf,
        #[arg(long, value_enum)]
        source: CostSourceArg,
        #[arg(long)]
        organization: String,
        #[arg(long)]
        repository: String,
        #[arg(long)]
        provenance: String,
        #[arg(long, default_value = "USD")]
        currency: String,
        #[arg(long)]
        model_mapping: Option<PathBuf>,
    },
}

#[derive(Debug, Parser)]
struct CostReportArgs {
    paths: Vec<PathBuf>,
    #[command(flatten)]
    common: CommonScanArgs,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum CostSourceArg {
    Normalized,
    Snowflake,
    Bigquery,
    Databricks,
    Trino,
    Generic,
}

impl From<CostSourceArg> for CostExportFormat {
    fn from(value: CostSourceArg) -> Self {
        match value {
            CostSourceArg::Normalized => Self::Normalized,
            CostSourceArg::Snowflake => Self::Snowflake,
            CostSourceArg::Bigquery => Self::BigQuery,
            CostSourceArg::Databricks => Self::Databricks,
            CostSourceArg::Trino => Self::Trino,
            CostSourceArg::Generic => Self::Generic,
        }
    }
}

#[derive(Debug, Parser)]
struct ExplainArgs {
    path: PathBuf,
    #[command(flatten)]
    common: CommonScanArgs,
    #[arg(long)]
    cost: bool,
}

#[derive(Debug, Parser)]
struct PrArgs {
    #[arg(long, default_value = "main")]
    base: String,
    #[command(flatten)]
    common: CommonScanArgs,
    #[arg(long)]
    fail_on: Option<String>,
    #[arg(long)]
    min_confidence: Option<String>,
    #[arg(long)]
    min_confidence_filter: bool,
    #[arg(long)]
    baseline: Option<PathBuf>,
    #[arg(long)]
    cost: bool,
    #[arg(long)]
    fail_on_cost_delta: Option<f64>,
    #[arg(long, value_name = "0.0..1.0")]
    min_cost_coverage: Option<f64>,
    #[command(flatten)]
    receipt: ReceiptArgs,
}

#[derive(Debug, Parser, Default)]
struct ReceiptArgs {
    #[arg(long)]
    summary_file: Option<PathBuf>,
    #[arg(long)]
    receipt_file: Option<PathBuf>,
    #[arg(long)]
    compare_receipt: Option<PathBuf>,
}

#[derive(Debug, Parser, Default)]
struct PolicyArgs {
    #[arg(long = "policy")]
    bundle: Option<PathBuf>,
    #[arg(long = "trust-store")]
    trust_store: Option<PathBuf>,
    #[arg(long = "policy-organization")]
    organization: Option<String>,
    #[arg(long = "policy-team")]
    team: Option<String>,
    #[arg(long = "policy-repository")]
    repository: Option<String>,
}

#[derive(Debug, Parser, Default)]
struct CommonScanArgs {
    #[arg(long)]
    warehouse: Option<String>,
    #[arg(long)]
    dialect: Option<String>,
    #[arg(long, value_enum)]
    format: Option<FormatArg>,
    #[arg(long)]
    manifest: Option<PathBuf>,
    #[arg(long)]
    base_manifest: Option<PathBuf>,
    #[arg(long)]
    analysis_policy: Option<String>,
    #[command(flatten)]
    policy: PolicyArgs,
}

impl CommonScanArgs {
    fn apply_common(&self, overrides: &mut ScanRuntimeOverrides) {
        overrides.warehouse = self.warehouse.clone();
        overrides.dialect = self.dialect.clone();
        overrides.format = self.format.map(Into::into);
        overrides.manifest_path = self.manifest.clone();
        overrides.base_manifest_path = self.base_manifest.clone();
        overrides.analysis_policy = self.analysis_policy.clone();
        overrides.policy_bundle_path = self.policy.bundle.clone();
        overrides.trust_store_path = self.policy.trust_store.clone();
        overrides.policy_organization = self.policy.organization.clone();
        overrides.policy_team = self.policy.team.clone();
        overrides.policy_repository = self.policy.repository.clone();
    }
}

#[derive(Debug, Parser)]
struct RulesArgs {
    #[arg(long, value_enum)]
    format: Option<FormatArg>,
}

#[derive(Debug, Parser)]
struct InitArgs {
    #[arg(long)]
    warehouse: Option<String>,
    #[arg(long)]
    force: bool,
    #[arg(long)]
    no_workflow: bool,
    #[arg(long)]
    no_config: bool,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum FormatArg {
    Text,
    Json,
    Github,
    Markdown,
    Sarif,
}

impl From<FormatArg> for OutputFormat {
    fn from(value: FormatArg) -> Self {
        match value {
            FormatArg::Text => OutputFormat::Text,
            FormatArg::Json => OutputFormat::Json,
            FormatArg::Github => OutputFormat::Github,
            FormatArg::Markdown => OutputFormat::Markdown,
            FormatArg::Sarif => OutputFormat::Sarif,
        }
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => ExitCode::from(code),
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::from(3)
        }
    }
}

fn run() -> Result<u8> {
    let cli = Cli::parse();
    match cli.command {
        Command::Scan(args) => {
            let config = match config_from_scan_args(&args).context("configuration error") {
                Ok(config) => config,
                Err(err) => return configuration_error(err),
            };
            scan_render_exit(&config, &args.receipt)
        }
        Command::Explain(args) => {
            let config = match config_from_explain_args(&args).context("configuration error") {
                Ok(config) => config,
                Err(err) => return configuration_error(err),
            };
            let result = explain(&config, &args.path)?;
            print!("{}", render(&result, config.format)?);
            Ok(if result.analysis.passed { 0 } else { 1 })
        }
        Command::Pr(args) => {
            let config = match config_from_pr_args(&args).context("configuration error") {
                Ok(config) => config,
                Err(err) => return configuration_error(err),
            };
            scan_render_exit(&config, &args.receipt)
        }
        Command::Cost(args) => run_cost_command(args.command),
        Command::Rules(args) => {
            let format = args
                .format
                .map(OutputFormat::from)
                .unwrap_or(OutputFormat::Text);
            print!("{}", render_rules(&rules(), format)?);
            Ok(0)
        }
        Command::Policy(args) => run_policy_command(args.command),
        Command::Init(args) => run_init_command(args),
    }
}

fn run_init_command(args: InitArgs) -> Result<u8> {
    let root = std::env::current_dir().context("failed to resolve current directory")?;
    let outcome = init_project(
        &root,
        &InitOptions {
            warehouse: args.warehouse,
            force: args.force,
            no_workflow: args.no_workflow,
            no_config: args.no_config,
        },
    )?;
    if outcome.created.is_empty() && outcome.skipped.is_empty() {
        println!("nothing to create (use --no-workflow / --no-config to narrow scope)");
    } else {
        println!("warehouse: {}", outcome.warehouse);
        for path in &outcome.created {
            println!("created {}", path.display());
        }
        for path in &outcome.skipped {
            println!(
                "skipped {} (already exists; use --force to overwrite)",
                path.display()
            );
        }
    }
    Ok(0)
}

fn run_cost_command(command: CostCommand) -> Result<u8> {
    match command {
        CostCommand::Report(args) => {
            let config = match config_from_cost_args(args).context("configuration error") {
                Ok(config) => config,
                Err(err) => return configuration_error(err),
            };
            let result = scan(&config)?;
            print!(
                "{}",
                costguard_output::render_cost_report(&result, config.format)?
            );
            Ok(0)
        }
        CostCommand::Normalize {
            input,
            output,
            source,
            organization,
            repository,
            provenance,
            currency,
            model_mapping,
        } => {
            let text = std::fs::read_to_string(&input)
                .with_context(|| format!("failed to read cost export {}", input.display()))?;
            let model_mapping = model_mapping
                .map(|path| read_json::<BTreeMap<String, String>>(&path, "model mapping"))
                .transpose()?
                .unwrap_or_default();
            let bundle = normalize_cost_export(
                &text,
                source.into(),
                &NormalizeCostOptions {
                    organization,
                    repository,
                    currency,
                    provenance,
                    model_mapping,
                },
            )?;
            write_json(&output, &bundle)?;
            println!(
                "normalized {} model cost observations to {}",
                bundle.observations.len(),
                output.display()
            );
            Ok(0)
        }
    }
}

fn run_policy_command(command: PolicyCommand) -> Result<u8> {
    match command {
        PolicyCommand::Keygen {
            key_id,
            private_key,
            trust_store,
            valid_until,
        } => {
            let now = chrono::Utc::now();
            let valid_until = valid_until
                .map(|value| {
                    chrono::DateTime::parse_from_rfc3339(&value)
                        .map(|value| value.with_timezone(&chrono::Utc))
                        .context("--valid-until must be RFC3339")
                })
                .transpose()?
                .unwrap_or_else(|| now + chrono::Duration::days(365));
            if valid_until <= now {
                anyhow::bail!("--valid-until must be in the future");
            }
            let key = generate_key(&key_id, now)?;
            write_private_json(&private_key, &key)?;
            let trust = TrustStoreV1 {
                version: 1,
                keys: vec![TrustedKeyV1 {
                    key_id: key.key_id.clone(),
                    algorithm: key.algorithm.clone(),
                    public_key: key.public_key.clone(),
                    valid_from: now.to_rfc3339(),
                    valid_until: valid_until.to_rfc3339(),
                    revoked: false,
                }],
            };
            write_json(&trust_store, &trust)?;
            println!(
                "created private key {} and trust store {}",
                private_key.display(),
                trust_store.display()
            );
            Ok(0)
        }
        PolicyCommand::Compile { input, output } => {
            let text = std::fs::read_to_string(&input)
                .with_context(|| format!("failed to read policy TOML {}", input.display()))?;
            let policy = compile_toml(&text)?;
            write_text(&output, &(canonical_json(&policy)? + "\n"))?;
            println!(
                "compiled {} ({})",
                output.display(),
                policy_digest(&policy)?
            );
            Ok(0)
        }
        PolicyCommand::Sign {
            policy,
            key,
            output,
        } => {
            let policy: PolicyDocumentV2 = read_json(&policy, "compiled policy")?;
            let key = read_json(&key, "private key")?;
            let signed = sign_policy(&policy, &key)?;
            write_json(&output, &signed)?;
            println!("signed {} with key {}", output.display(), signed.key_id);
            Ok(0)
        }
        PolicyCommand::Verify {
            bundle,
            trust_store,
        } => {
            let signed = read_signed_policy(&bundle)?;
            let trust = read_trust_store(&trust_store)?;
            let policy = verify_policy(&signed, &trust, chrono::Utc::now())?;
            println!(
                "verified policy {} version {} ({})",
                policy.id,
                policy.version,
                policy_digest(&policy)?
            );
            Ok(0)
        }
        PolicyCommand::Resolve {
            bundle,
            trust_store,
            organization,
            repository,
            team,
            path,
        } => {
            let signed = read_signed_policy(&bundle)?;
            let trust = read_trust_store(&trust_store)?;
            let policy = verify_policy(&signed, &trust, chrono::Utc::now())?;
            let resolved = resolve_policy(
                &policy,
                &ResolutionContext {
                    organization: &organization,
                    team: team.as_deref(),
                    repository: &repository,
                    path: path.as_deref(),
                },
            )?;
            println!("{}", serde_json::to_string_pretty(&resolved)?);
            Ok(0)
        }
        PolicyCommand::Inspect {
            bundle,
            trust_store,
        } => {
            let signed = read_signed_policy(&bundle)?;
            let trust = read_trust_store(&trust_store)?;
            let policy = verify_policy(&signed, &trust, chrono::Utc::now())?;
            println!("id: {}", policy.id);
            println!("version: {}", policy.version);
            println!("organization: {}", policy.organization);
            println!("digest: {}", policy_digest(&policy)?);
            println!("signing_key: {}", signed.key_id);
            println!("expires_at: {}", policy.expires_at);
            println!("scopes: {}", policy.scopes.len());
            println!(
                "custom_rules: {}",
                policy
                    .scopes
                    .iter()
                    .map(|scope| scope.custom_rules.len())
                    .sum::<usize>()
            );
            println!(
                "exceptions: {}",
                policy
                    .scopes
                    .iter()
                    .map(|scope| scope.exceptions.len())
                    .sum::<usize>()
            );
            Ok(0)
        }
    }
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path, label: &str) -> Result<T> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {label} {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("invalid {label} {}", path.display()))
}

fn write_json<T: serde::Serialize>(path: &Path, value: &T) -> Result<()> {
    write_text(path, &(serde_json::to_string_pretty(value)? + "\n"))
}

fn write_text(path: &Path, text: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
    }
    std::fs::write(path, text).with_context(|| format!("failed to write {}", path.display()))
}

fn write_private_json<T: serde::Serialize>(path: &Path, value: &T) -> Result<()> {
    write_json(path, value)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .with_context(|| format!("failed to secure {}", path.display()))?;
    }
    Ok(())
}

fn configuration_error(err: anyhow::Error) -> Result<u8> {
    eprintln!("error: {err:#}");
    Ok(2)
}

fn base_config() -> Result<ScanConfig> {
    let root = std::env::current_dir().context("failed to resolve current directory")?;
    let config = ScanConfig {
        root: root.clone(),
        ..ScanConfig::default()
    };
    let file_config = load_config(&root)?;
    apply_file_config(config, file_config)
}

fn scan_render_exit(config: &ScanConfig, receipt: &ReceiptArgs) -> Result<u8> {
    validate_receipt_args(receipt)?;
    let mut result = scan(config)?;
    if receipt.receipt_file.is_some() || receipt.compare_receipt.is_some() {
        result.pr_summary.get_or_insert_with(PrSummary::default);
    }
    if let Some(path) = &receipt.compare_receipt {
        let trend = compare_receipt(path, &result)?;
        result
            .pr_summary
            .get_or_insert_with(PrSummary::default)
            .trend = Some(trend);
    }
    if let Some(path) = &receipt.summary_file {
        write_text(path, &(render(&result, OutputFormat::Markdown)? + "\n"))?;
    }
    if let Some(path) = &receipt.receipt_file {
        write_text(path, &(render(&result, OutputFormat::Json)? + "\n"))?;
    }
    print!("{}", render(&result, config.format)?);
    Ok(
        if result.should_fail(
            config.fail_on,
            config.min_confidence,
            fail_on_monthly_delta(config),
            fail_on_monthly_delta_gb(config),
        ) {
            1
        } else {
            0
        },
    )
}

fn config_from_scan_args(args: &ScanArgs) -> Result<ScanConfig> {
    let mut config = base_config()?;
    if !args.paths.is_empty() {
        config.paths = args.paths.clone();
    }
    let mut overrides = ScanRuntimeOverrides::default();
    args.common.apply_common(&mut overrides);
    overrides.fail_on = args.fail_on.clone();
    overrides.min_confidence = args.min_confidence.clone();
    if args.min_confidence_filter {
        overrides.min_confidence_filter = Some(true);
    }
    overrides.baseline_path = args.baseline.clone();
    overrides.write_baseline_path = args.write_baseline.clone();
    overrides.cost = args.cost;
    overrides.fail_on_cost_delta = args.fail_on_cost_delta;
    overrides.min_cost_coverage = args.min_cost_coverage;
    overrides.apply_to(&mut config)?;
    validate_scan_config(&config)?;
    Ok(config)
}

fn config_from_explain_args(args: &ExplainArgs) -> Result<ScanConfig> {
    let mut config = base_config()?;
    let mut overrides = ScanRuntimeOverrides::default();
    args.common.apply_common(&mut overrides);
    overrides.cost = args.cost;
    overrides.apply_to(&mut config)?;
    validate_scan_config(&config)?;
    Ok(config)
}

fn config_from_cost_args(args: CostReportArgs) -> Result<ScanConfig> {
    let mut config = base_config()?;
    if !args.paths.is_empty() {
        config.paths = args.paths;
    }
    let mut overrides = ScanRuntimeOverrides::default();
    args.common.apply_common(&mut overrides);
    overrides.cost = true;
    overrides.apply_to(&mut config)?;
    validate_scan_config(&config)?;
    Ok(config)
}

fn config_from_pr_args(args: &PrArgs) -> Result<ScanConfig> {
    let mut config = base_config()?;
    config.changed_only = true;
    config.base_branch = Some(args.base.clone());
    let mut overrides = ScanRuntimeOverrides::default();
    args.common.apply_common(&mut overrides);
    overrides.fail_on = args.fail_on.clone();
    overrides.min_confidence = args.min_confidence.clone();
    if args.min_confidence_filter {
        overrides.min_confidence_filter = Some(true);
    }
    overrides.baseline_path = args.baseline.clone();
    overrides.cost = args.cost;
    overrides.fail_on_cost_delta = args.fail_on_cost_delta;
    overrides.min_cost_coverage = args.min_cost_coverage;
    overrides.apply_to(&mut config)?;
    validate_scan_config(&config)?;
    Ok(config)
}

fn validate_receipt_args(args: &ReceiptArgs) -> Result<()> {
    if args
        .summary_file
        .as_ref()
        .is_some_and(|summary| args.receipt_file.as_ref() == Some(summary))
    {
        anyhow::bail!("--summary-file and --receipt-file must use different paths");
    }
    for (flag, path) in [
        ("--summary-file", args.summary_file.as_ref()),
        ("--receipt-file", args.receipt_file.as_ref()),
    ] {
        if path.is_some_and(|path| path.is_dir()) {
            anyhow::bail!("{flag} path cannot be a directory");
        }
    }
    if let Some(path) = &args.compare_receipt {
        if !path.is_file() {
            anyhow::bail!("--compare-receipt path does not exist: {}", path.display());
        }
    }
    Ok(())
}

fn compare_receipt(path: &Path, current: &costguard_core::ScanResult) -> Result<ReceiptTrend> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read receipt {}", path.display()))?;
    let previous: serde_json::Value = serde_json::from_str(&text)
        .with_context(|| format!("invalid receipt JSON {}", path.display()))?;
    if previous
        .get("schema_version")
        .and_then(|value| value.as_u64())
        != Some(4)
    {
        anyhow::bail!("receipt {} must use JSON schema version 4", path.display());
    }
    let previous_diagnostics = previous
        .get("diagnostics")
        .and_then(|value| value.as_array())
        .context("receipt diagnostics must be an array")?;
    let previous_high = previous_diagnostics
        .iter()
        .filter(|diagnostic| {
            matches!(
                diagnostic.get("severity").and_then(|value| value.as_str()),
                Some("high" | "critical")
            )
        })
        .count();
    let current_high = current
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity >= costguard_diagnostics::Severity::High)
        .count();
    let previous_usd = previous
        .pointer("/cost/savings_p50_usd")
        .and_then(|value| value.as_f64());
    let current_usd = current
        .cost_summary
        .as_ref()
        .map(|cost| cost.savings_p50_usd);
    let previous_gb = previous
        .pointer("/cost/savings_gb_months")
        .and_then(|value| value.as_f64());
    let current_gb = current
        .cost_summary
        .as_ref()
        .map(|cost| cost.savings_gb_months);
    Ok(ReceiptTrend {
        diagnostics_delta: current.diagnostics.len() as i64 - previous_diagnostics.len() as i64,
        high_findings_delta: current_high as i64 - previous_high as i64,
        monthly_savings_delta_usd: delta(previous_usd, current_usd),
        monthly_savings_delta_gb: delta(previous_gb, current_gb),
    })
}

fn delta(previous: Option<f64>, current: Option<f64>) -> Option<f64> {
    (previous.is_some() || current.is_some())
        .then(|| current.unwrap_or(0.0) - previous.unwrap_or(0.0))
}

fn fail_on_monthly_delta(config: &ScanConfig) -> Option<f64> {
    config
        .cost
        .as_ref()
        .and_then(|cost| cost.fail_on_monthly_delta)
}

fn fail_on_monthly_delta_gb(config: &ScanConfig) -> Option<f64> {
    config
        .cost
        .as_ref()
        .and_then(|cost| cost.fail_on_monthly_delta_gb)
}
