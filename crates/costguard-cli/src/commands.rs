//! Costguard command dispatch and handlers.
//!
//! Clap definitions are kept separately from the thin binary entry point.
//! Scan orchestration delegates to [`costguard_core`].

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use costguard_core::{
    capture_rocky_artifact, doctor_resolved, explain_resolved, init_project, load_resolved_config,
    rules, scan_resolved, validate_resolved_scan_request, write_rocky_artifact, DoctorReport,
    DoctorStatus, InitOptions, OutputFormat, PrSummary, ReceiptTrend, ResolvedScanRequest,
    RockyCaptureOptions, ScanConfig, ScanRuntimeOverrides,
};
use costguard_cost::{normalize_cost_export, CostExportFormat, NormalizeCostOptions};
use costguard_output::{receipt_trend, render, render_rules};
use costguard_policy::{
    canonical_json, compile_toml, generate_key, policy_digest, read_signed_policy,
    read_trust_store, resolve_policy, sign_policy, verify_policy, PolicyDocumentV2,
    ResolutionContext, TrustStoreV1, TrustedKeyV1,
};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[path = "args.rs"]
mod args;
use args::Cli;

#[derive(Debug, Subcommand)]
enum Command {
    /// Scan a project and report cost-risk findings.
    Scan(ScanArgs),
    /// Explain findings for one file.
    Explain(ExplainArgs),
    /// Scan files changed against a git base revision.
    Pr(PrArgs),
    /// Report or normalize model cost data.
    Cost(CostArgs),
    /// Capture and validate Rocky compile metadata.
    Rocky(RockyArgs),
    /// List the built-in rule catalog.
    Rules(RulesArgs),
    /// Create, sign, verify, and inspect managed policy bundles.
    Policy(PolicyCommandArgs),
    /// Generate a starter configuration and GitHub workflow.
    Init(InitArgs),
    /// Check whether a project is ready for local and PR scans.
    Doctor(DoctorArgs),
}

#[derive(Debug, Parser)]
struct PolicyCommandArgs {
    #[command(subcommand)]
    command: PolicyCommand,
}

#[derive(Debug, Subcommand)]
enum PolicyCommand {
    /// Generate a private signing key and matching trust store.
    Keygen {
        /// Stable identifier embedded in signatures.
        key_id: String,
        /// Destination for the private key JSON.
        #[arg(long)]
        private_key: PathBuf,
        /// Destination for the public trust store JSON.
        #[arg(long)]
        trust_store: PathBuf,
        /// RFC3339 public-key expiry; defaults to one year.
        #[arg(long)]
        valid_until: Option<String>,
        /// Replace existing regular output files.
        #[arg(long)]
        force: bool,
    },
    /// Compile policy TOML into canonical policy JSON.
    Compile {
        /// Policy TOML source.
        input: PathBuf,
        /// Canonical policy JSON destination.
        output: PathBuf,
    },
    /// Sign compiled policy JSON.
    Sign {
        /// Compiled policy JSON.
        policy: PathBuf,
        /// Private signing key JSON.
        key: PathBuf,
        /// Signed policy bundle destination.
        output: PathBuf,
    },
    /// Verify a signed policy bundle against a trust store.
    Verify {
        /// Signed policy bundle.
        bundle: PathBuf,
        /// Public trust store.
        trust_store: PathBuf,
    },
    /// Resolve the effective policy for an organization and repository.
    Resolve {
        /// Signed policy bundle.
        bundle: PathBuf,
        /// Public trust store.
        trust_store: PathBuf,
        /// Organization slug used for scope resolution.
        #[arg(long)]
        organization: String,
        /// Repository slug in owner/name form.
        #[arg(long)]
        repository: String,
        /// Optional team scope.
        #[arg(long)]
        team: Option<String>,
        /// Optional repository path scope.
        #[arg(long)]
        path: Option<String>,
    },
    /// Print verified policy metadata and counts.
    Inspect {
        /// Signed policy bundle.
        bundle: PathBuf,
        /// Public trust store.
        trust_store: PathBuf,
    },
}

#[derive(Debug, Parser)]
#[command(
    after_help = "Examples:\n  costguard scan\n  costguard scan models --cost --format markdown"
)]
struct ScanArgs {
    /// Files or directories to scan; defaults to the configured project.
    paths: Vec<PathBuf>,
    #[command(flatten)]
    common: CommonScanArgs,
    /// Minimum severity that returns exit code 1.
    #[arg(long)]
    fail_on: Option<String>,
    /// Minimum confidence eligible for the failure gate.
    #[arg(long)]
    min_confidence: Option<String>,
    /// Hide diagnostics below --min-confidence from output.
    #[arg(long)]
    min_confidence_filter: bool,
    /// Existing finding baseline to apply.
    #[arg(long)]
    baseline: Option<PathBuf>,
    /// Write the current findings as a baseline.
    #[arg(long)]
    write_baseline: Option<PathBuf>,
    /// Enable project cost estimation.
    #[arg(long)]
    cost: bool,
    /// Fail when addressable mapped USD reaches this monthly threshold.
    #[arg(long)]
    fail_on_cost_delta: Option<f64>,
    /// Require this mapped-spend coverage fraction.
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
    /// Render the project-level cost report.
    Report(CostReportArgs),
    /// Convert a supported warehouse or pipeline export to Costguard observations.
    Normalize {
        /// Source export to read.
        input: PathBuf,
        /// Normalized JSON bundle to write.
        output: PathBuf,
        /// Input export format.
        #[arg(long, value_enum)]
        source: CostSourceArg,
        /// Organization recorded in the normalized bundle.
        #[arg(long)]
        organization: String,
        /// Repository recorded in the normalized bundle.
        #[arg(long)]
        repository: String,
        /// Human-readable source provenance.
        #[arg(long)]
        provenance: String,
        /// Currency code; currently USD.
        #[arg(long, default_value = "USD")]
        currency: String,
        /// Optional JSON map from source identifiers to dbt model IDs.
        #[arg(long)]
        model_mapping: Option<PathBuf>,
    },
}

#[derive(Debug, Parser)]
struct RockyArgs {
    #[command(subcommand)]
    command: RockyCommand,
}

#[derive(Debug, Subcommand)]
enum RockyCommand {
    /// Seal Rocky compile JSON for reproducible Costguard analysis.
    Capture {
        /// Rocky `compile --output json --expand-macros` output.
        #[arg(long = "compile")]
        compile_path: PathBuf,
        /// Rocky project configuration.
        #[arg(long)]
        rocky_config: Option<PathBuf>,
        /// Rocky models directory.
        #[arg(long)]
        models_dir: Option<PathBuf>,
        /// Additional compile input file or directory to seal.
        #[arg(long = "input")]
        extra_inputs: Vec<PathBuf>,
        /// Destination for the sealed Costguard artifact.
        #[arg(long)]
        output: Option<PathBuf>,
    },
}

#[derive(Debug, Parser)]
struct CostReportArgs {
    /// Files or directories to include; defaults to the configured project.
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
    Pipeline,
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
            CostSourceArg::Pipeline => Self::Pipeline,
            CostSourceArg::Generic => Self::Generic,
        }
    }
}

#[derive(Debug, Parser)]
struct ExplainArgs {
    /// SQL or dbt file to analyze.
    path: PathBuf,
    #[command(flatten)]
    common: CommonScanArgs,
    /// Attach advisory cost estimates.
    #[arg(long)]
    cost: bool,
}

#[derive(Debug, Parser)]
#[command(after_help = "Example:\n  costguard pr --base origin/main --block-only-new=true")]
struct PrArgs {
    /// Git revision used as the comparison base.
    #[arg(long, default_value = "main")]
    base: String,
    #[command(flatten)]
    common: CommonScanArgs,
    /// Minimum severity that returns exit code 1.
    #[arg(long)]
    fail_on: Option<String>,
    /// Minimum confidence eligible for the failure gate.
    #[arg(long)]
    min_confidence: Option<String>,
    /// Hide diagnostics below --min-confidence from output.
    #[arg(long)]
    min_confidence_filter: bool,
    /// Existing finding baseline to apply.
    #[arg(long)]
    baseline: Option<PathBuf>,
    /// Enable project cost estimation.
    #[arg(long)]
    cost: bool,
    /// Fail when addressable mapped USD reaches this monthly threshold.
    #[arg(long)]
    fail_on_cost_delta: Option<f64>,
    /// Enforce only introduced or regressed findings.
    #[arg(
        long,
        num_args = 0..=1,
        default_missing_value = "true",
        require_equals = true
    )]
    block_only_new: Option<bool>,
    /// Fail when mapped project cost rises by this USD/month amount.
    #[arg(long, value_name = "USD_PER_MONTH")]
    fail_on_pr_cost_increase: Option<f64>,
    /// Require this mapped-spend coverage fraction.
    #[arg(long, value_name = "0.0..1.0")]
    min_cost_coverage: Option<f64>,
    #[command(flatten)]
    receipt: ReceiptArgs,
}

#[derive(Debug, Parser, Default)]
struct ReceiptArgs {
    /// Write the Markdown summary to this file.
    #[arg(long)]
    summary_file: Option<PathBuf>,
    /// Write the JSON v4 receipt to this file.
    #[arg(long)]
    receipt_file: Option<PathBuf>,
    /// Compare against a previous JSON v4 receipt.
    #[arg(long)]
    compare_receipt: Option<PathBuf>,
}

#[derive(Debug, Parser, Default)]
struct PolicyArgs {
    /// Signed policy bundle.
    #[arg(long = "policy")]
    bundle: Option<PathBuf>,
    /// Public trust store for policy verification.
    #[arg(long = "trust-store")]
    trust_store: Option<PathBuf>,
    /// Organization used for policy scope resolution.
    #[arg(long = "policy-organization")]
    organization: Option<String>,
    /// Optional team used for policy scope resolution.
    #[arg(long = "policy-team")]
    team: Option<String>,
    /// Repository used for policy scope resolution.
    #[arg(long = "policy-repository")]
    repository: Option<String>,
}

#[derive(Debug, Parser, Default)]
struct CommonScanArgs {
    /// Warehouse platform used for dialect and rule context.
    #[arg(long)]
    warehouse: Option<String>,
    /// SQL dialect override.
    #[arg(long)]
    dialect: Option<String>,
    /// Output format.
    #[arg(long, value_enum)]
    format: Option<FormatArg>,
    /// dbt manifest path relative to the project root.
    #[arg(long)]
    manifest: Option<PathBuf>,
    /// Base-branch manifest used by PR analysis.
    #[arg(long)]
    base_manifest: Option<PathBuf>,
    /// Sealed Rocky compile artifact for the current checkout.
    #[arg(long)]
    rocky_artifact: Option<PathBuf>,
    /// Sealed Rocky compile artifact for PR base comparison.
    #[arg(long)]
    base_rocky_artifact: Option<PathBuf>,
    /// Analysis completeness policy: standard or strict.
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
        overrides.rocky_artifact_path = self.rocky_artifact.clone();
        overrides.base_rocky_artifact_path = self.base_rocky_artifact.clone();
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
    /// Rule catalog output format.
    #[arg(long, value_enum)]
    format: Option<FormatArg>,
}

#[derive(Debug, Parser)]
#[command(after_help = "Examples:\n  costguard init\n  costguard init --dbt-dir analytics")]
struct InitArgs {
    /// Warehouse platform to write, overriding auto-detection.
    #[arg(long)]
    warehouse: Option<String>,
    /// Initialization profile; currently local-duckdb.
    #[arg(long)]
    profile: Option<String>,
    /// dbt project directory relative to the repository root.
    #[arg(long)]
    dbt_dir: Option<PathBuf>,
    /// Replace generated files that already exist.
    #[arg(long)]
    force: bool,
    /// Do not generate a GitHub workflow.
    #[arg(long)]
    no_workflow: bool,
    /// Do not generate costguard.toml.
    #[arg(long)]
    no_config: bool,
}

#[derive(Debug, Parser)]
#[command(after_help = "Example:\n  costguard doctor --dbt-dir analytics")]
struct DoctorArgs {
    /// dbt project directory relative to the repository root.
    #[arg(long)]
    dbt_dir: Option<PathBuf>,
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

pub(crate) fn main_entry() -> ExitCode {
    match run() {
        Ok(code) => ExitCode::from(code),
        Err(err)
            if err
                .downcast_ref::<costguard_core::ProjectConfigurationError>()
                .is_some() =>
        {
            eprintln!("error: configuration error: {err:#}");
            ExitCode::from(2)
        }
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
            let result = explain_resolved(&config, &args.path)?;
            print!("{}", render(&result, config.config.format)?);
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
        Command::Rocky(args) => run_rocky_command(args.command),
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
        Command::Doctor(args) => run_doctor_command(args),
    }
}

fn run_rocky_command(command: RockyCommand) -> Result<u8> {
    match command {
        RockyCommand::Capture {
            compile_path,
            rocky_config,
            models_dir,
            extra_inputs,
            output,
        } => {
            let root = std::env::current_dir().context("failed to resolve current directory")?;
            let request = load_resolved_config(
                &root,
                ScanConfig {
                    root: root.clone(),
                    ..ScanConfig::default()
                },
            )?;
            let rocky_config = rocky_config.unwrap_or_else(|| request.rocky.config_path.clone());
            let models_dir = models_dir.unwrap_or_else(|| request.rocky.models_dir.clone());
            let output = output
                .or_else(|| request.rocky.artifact_path.clone())
                .unwrap_or_else(|| PathBuf::from("target/costguard-rocky.json"));
            let artifact = capture_rocky_artifact(&RockyCaptureOptions {
                root,
                compile_path,
                rocky_config,
                models_dir,
                extra_inputs,
                max_artifact_bytes: request.rocky.max_artifact_bytes,
            })?;
            write_rocky_artifact(&output, &artifact)?;
            println!(
                "captured {} Rocky models at {}",
                artifact.source_map.len(),
                output.display()
            );
            Ok(0)
        }
    }
}

fn run_init_command(args: InitArgs) -> Result<u8> {
    let root = std::env::current_dir().context("failed to resolve current directory")?;
    let dbt_dir = args.dbt_dir.clone();
    let outcome = init_project(
        &root,
        &InitOptions {
            warehouse: args.warehouse,
            profile: args.profile,
            dbt_dir: args.dbt_dir,
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
    match doctor_config(&root, dbt_dir.as_deref())
        .and_then(|request| doctor_resolved(&request, &root))
    {
        Ok(report) => print_doctor_report(&report),
        Err(error) => eprintln!("warning: readiness check could not complete: {error:#}"),
    }
    Ok(0)
}

fn run_doctor_command(args: DoctorArgs) -> Result<u8> {
    let root = std::env::current_dir().context("failed to resolve current directory")?;
    let config = match doctor_config(&root, args.dbt_dir.as_deref()).context("configuration error")
    {
        Ok(config) => config,
        Err(error) => return configuration_error(error),
    };
    let report = doctor_resolved(&config, &root)?;
    print_doctor_report(&report);
    Ok(if report.has_blockers() { 1 } else { 0 })
}

fn doctor_config(repository_root: &Path, dbt_dir: Option<&Path>) -> Result<ResolvedScanRequest> {
    let project_root = dbt_dir
        .map(|path| repository_root.join(path))
        .unwrap_or_else(|| repository_root.to_path_buf());
    resolve_scan_request(
        &project_root,
        CommandPatch::default(),
        ScanRuntimeOverrides::default(),
    )
}

fn print_doctor_report(report: &DoctorReport) {
    println!("Costguard doctor");
    for check in &report.checks {
        let status = match check.status {
            DoctorStatus::Pass => "pass",
            DoctorStatus::Warn => "warn",
            DoctorStatus::Fail => "fail",
        };
        println!("[{status}] {}: {}", check.name, check.message);
    }
    if report.has_blockers() {
        println!("Not ready: resolve the failed checks above.");
    } else {
        println!("Ready: no blocking setup issues found.");
    }
}

fn run_cost_command(command: CostCommand) -> Result<u8> {
    match command {
        CostCommand::Report(args) => {
            let config = match config_from_cost_args(args).context("configuration error") {
                Ok(config) => config,
                Err(err) => return configuration_error(err),
            };
            let result = scan_resolved(&config)?;
            print!(
                "{}",
                costguard_output::render_cost_report(&result, config.config.format)?
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
            force,
        } => {
            validate_keygen_destinations(&private_key, &trust_store, force)?;
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
            persist_keygen_outputs(&private_key, &key, &trust_store, &trust, force)?;
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

fn validate_keygen_destinations(private_key: &Path, trust_store: &Path, force: bool) -> Result<()> {
    if normalize_destination(private_key)? == normalize_destination(trust_store)? {
        anyhow::bail!("private key and trust store paths must be different");
    }
    for (label, path) in [("private key", private_key), ("trust store", trust_store)] {
        match std::fs::symlink_metadata(path) {
            Ok(metadata) if !force => {
                anyhow::bail!(
                    "{label} output already exists: {} (use --force to replace it)",
                    path.display()
                );
            }
            Ok(metadata) if !metadata.file_type().is_file() => {
                anyhow::bail!(
                    "refusing to replace non-regular {label} output: {}",
                    path.display()
                );
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| format!("failed to inspect {}", path.display()));
            }
        }
    }
    Ok(())
}

fn normalize_destination(path: &Path) -> Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    Ok(normalized)
}

fn persist_keygen_outputs<K: serde::Serialize, T: serde::Serialize>(
    private_key_path: &Path,
    private_key: &K,
    trust_store_path: &Path,
    trust_store: &T,
    force: bool,
) -> Result<()> {
    let private_bytes = serde_json::to_vec_pretty(private_key)?;
    let trust_bytes = serde_json::to_vec_pretty(trust_store)?;
    let private_temp = prepare_keygen_temp(private_key_path, &private_bytes, true)?;
    let trust_temp = prepare_keygen_temp(trust_store_path, &trust_bytes, false)?;
    persist_keygen_temp(private_temp, private_key_path, force)?;
    persist_keygen_temp(trust_temp, trust_store_path, force)?;
    Ok(())
}

fn prepare_keygen_temp(
    path: &Path,
    bytes: &[u8],
    private: bool,
) -> Result<tempfile::NamedTempFile> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)
        .with_context(|| format!("failed to create {}", parent.display()))?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)
        .with_context(|| format!("failed to create temporary output in {}", parent.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = if private { 0o600 } else { 0o644 };
        temp.as_file()
            .set_permissions(std::fs::Permissions::from_mode(mode))
            .with_context(|| format!("failed to set permissions for {}", path.display()))?;
    }
    temp.write_all(bytes)
        .with_context(|| format!("failed to write temporary output for {}", path.display()))?;
    temp.write_all(b"\n")
        .with_context(|| format!("failed to write temporary output for {}", path.display()))?;
    temp.as_file_mut()
        .sync_all()
        .with_context(|| format!("failed to flush temporary output for {}", path.display()))?;
    Ok(temp)
}

fn persist_keygen_temp(temp: tempfile::NamedTempFile, path: &Path, force: bool) -> Result<()> {
    let result = if force {
        temp.persist(path)
    } else {
        temp.persist_noclobber(path)
    };
    result
        .map(|_| ())
        .map_err(|error| error.error)
        .with_context(|| format!("failed to persist {}", path.display()))
}

fn configuration_error(err: anyhow::Error) -> Result<u8> {
    eprintln!("error: {err:#}");
    Ok(2)
}

#[derive(Default)]
struct CommandPatch {
    paths: Option<Vec<PathBuf>>,
    changed_only: bool,
    base_branch: Option<String>,
}

fn resolve_scan_request(
    root: &Path,
    patch: CommandPatch,
    overrides: ScanRuntimeOverrides,
) -> Result<ResolvedScanRequest> {
    let mut request = load_resolved_config(
        root,
        ScanConfig {
            root: root.to_path_buf(),
            ..ScanConfig::default()
        },
    )?;
    if let Some(paths) = patch.paths {
        request.config.paths = paths;
    }
    if patch.changed_only {
        request.config.changed_only = true;
        request.config.base_branch = patch.base_branch;
    }
    overrides.apply_to_request(&mut request)?;
    validate_resolved_scan_request(&request)?;
    Ok(request)
}

fn resolve_current_request(
    patch: CommandPatch,
    overrides: ScanRuntimeOverrides,
) -> Result<ResolvedScanRequest> {
    let root = std::env::current_dir().context("failed to resolve current directory")?;
    resolve_scan_request(&root, patch, overrides)
}

fn scan_render_exit(config: &ResolvedScanRequest, receipt: &ReceiptArgs) -> Result<u8> {
    validate_receipt_args(receipt)?;
    let mut result = scan_resolved(config)?;
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
    print!("{}", render(&result, config.config.format)?);
    Ok(
        if result.should_fail(
            config.config.fail_on,
            config.config.min_confidence,
            fail_on_monthly_delta(&config.config),
            fail_on_monthly_delta_gb(&config.config),
        ) {
            1
        } else {
            0
        },
    )
}

fn config_from_scan_args(args: &ScanArgs) -> Result<ResolvedScanRequest> {
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
    resolve_current_request(
        CommandPatch {
            paths: (!args.paths.is_empty()).then(|| args.paths.clone()),
            ..CommandPatch::default()
        },
        overrides,
    )
}

fn config_from_explain_args(args: &ExplainArgs) -> Result<ResolvedScanRequest> {
    let mut overrides = ScanRuntimeOverrides::default();
    args.common.apply_common(&mut overrides);
    overrides.cost = args.cost;
    resolve_current_request(CommandPatch::default(), overrides)
}

fn config_from_cost_args(args: CostReportArgs) -> Result<ResolvedScanRequest> {
    let mut overrides = ScanRuntimeOverrides::default();
    args.common.apply_common(&mut overrides);
    overrides.cost = true;
    resolve_current_request(
        CommandPatch {
            paths: (!args.paths.is_empty()).then_some(args.paths),
            ..CommandPatch::default()
        },
        overrides,
    )
}

fn config_from_pr_args(args: &PrArgs) -> Result<ResolvedScanRequest> {
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
    overrides.block_only_new = args.block_only_new;
    overrides.fail_on_pr_cost_increase = args.fail_on_pr_cost_increase;
    resolve_current_request(
        CommandPatch {
            changed_only: true,
            base_branch: Some(args.base.clone()),
            ..CommandPatch::default()
        },
        overrides,
    )
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
    receipt_trend(&text, current)
        .with_context(|| format!("invalid receipt JSON {}", path.display()))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_resolver_applies_file_patch_and_runtime_precedence_once() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join("costguard.toml"),
            r#"
warehouse = "bigquery"
[scan]
paths = ["configured"]
max_total_base_bytes = 4096
[cost]
enabled = true
"#,
        )
        .unwrap();
        let overrides = ScanRuntimeOverrides {
            warehouse: Some("snowflake".into()),
            ..ScanRuntimeOverrides::default()
        };

        let request = resolve_scan_request(
            root.path(),
            CommandPatch {
                paths: Some(vec![PathBuf::from("command")]),
                changed_only: true,
                base_branch: Some("origin/main".into()),
            },
            overrides,
        )
        .unwrap();

        assert_eq!(request.config.paths, vec![PathBuf::from("command")]);
        assert_eq!(request.config.platform.to_string(), "snowflake");
        assert_eq!(
            request.config.cost.as_ref().map(|cost| cost.warehouse),
            Some(costguard_cost::Warehouse::Snowflake)
        );
        assert!(request.config.changed_only);
        assert_eq!(request.config.base_branch.as_deref(), Some("origin/main"));
        assert_eq!(request.max_total_base_bytes, 4096);
    }
}
