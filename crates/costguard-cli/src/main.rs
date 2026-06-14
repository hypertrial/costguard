//! Costguard CLI binary.
//!
//! Entry point for `scan`, `explain`, `pr`, `cost`, `rules`, `baseline`, and
//! `policy` subcommands. Delegates scan orchestration to
//! [`costguard_core`].

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use costguard_core::{
    apply_file_config, explain, generate_identity_map, load_baseline_v2, load_config,
    load_identity_map, migrate_baseline_v2, rules, scan, scan_for_identity_map,
    validate_scan_config, OutputFormat, ScanConfig, ScanRuntimeOverrides,
};
use costguard_cost::{normalize_cost_export, CostExportFormat, NormalizeCostOptions};
use costguard_output::{render, render_rules};
use costguard_policy::{
    canonical_json, compile_toml, generate_key, load_policy_v1_json, migrate_policy_v1,
    policy_digest, read_signed_policy, read_trust_store, resolve_policy, sign_policy,
    verify_policy, PolicyDocumentV2, ResolutionContext, TrustStoreV1, TrustedKeyV1,
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
    Baseline(BaselineArgs),
    Policy(PolicyCommandArgs),
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
    MigrateV1 {
        input: PathBuf,
        map: PathBuf,
        output: PathBuf,
        #[arg(long)]
        version: String,
        #[arg(long = "issued-at")]
        issued_at: String,
        #[arg(long = "expires-at")]
        expires_at: Option<String>,
        #[arg(long = "trust-store")]
        trust_store: Option<PathBuf>,
    },
}

#[derive(Debug, Parser)]
struct BaselineArgs {
    #[command(subcommand)]
    command: BaselineCommand,
}

#[derive(Debug, Subcommand)]
enum BaselineCommand {
    MigrateV1 {
        input: PathBuf,
        output: PathBuf,
        #[arg(long)]
        warehouse: Option<String>,
        #[arg(long)]
        manifest: Option<PathBuf>,
        paths: Vec<PathBuf>,
    },
    IdentityMapV2 {
        output: PathBuf,
        #[arg(long)]
        warehouse: Option<String>,
        #[arg(long)]
        manifest: Option<PathBuf>,
        paths: Vec<PathBuf>,
        #[command(flatten)]
        policy: PolicyArgs,
    },
    MigrateV2 {
        input: PathBuf,
        map: PathBuf,
        output: PathBuf,
        #[arg(long = "policy")]
        policy: Option<PathBuf>,
        #[arg(long = "trust-store")]
        trust_store: Option<PathBuf>,
    },
}

#[derive(Debug, Parser)]
struct ScanArgs {
    paths: Vec<PathBuf>,
    #[arg(long)]
    warehouse: Option<String>,
    #[arg(long)]
    dialect: Option<String>,
    #[arg(long, value_enum)]
    format: Option<FormatArg>,
    #[arg(long)]
    manifest: Option<PathBuf>,
    #[arg(long)]
    fail_on: Option<String>,
    #[arg(long)]
    min_confidence: Option<String>,
    #[arg(long)]
    baseline: Option<PathBuf>,
    #[arg(long)]
    write_baseline: Option<PathBuf>,
    #[arg(long)]
    cost: bool,
    #[arg(long)]
    fail_on_cost_delta: Option<f64>,
    #[arg(long)]
    analysis_policy: Option<String>,
    #[command(flatten)]
    policy: PolicyArgs,
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
    #[arg(long)]
    warehouse: Option<String>,
    #[arg(long)]
    dialect: Option<String>,
    #[arg(long, value_enum)]
    format: Option<FormatArg>,
    #[arg(long)]
    manifest: Option<PathBuf>,
    #[arg(long)]
    analysis_policy: Option<String>,
    #[command(flatten)]
    policy: PolicyArgs,
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
    #[arg(long)]
    warehouse: Option<String>,
    #[arg(long)]
    dialect: Option<String>,
    #[arg(long, value_enum)]
    format: Option<FormatArg>,
    #[arg(long)]
    manifest: Option<PathBuf>,
    #[arg(long)]
    cost: bool,
    #[arg(long)]
    analysis_policy: Option<String>,
    #[command(flatten)]
    policy: PolicyArgs,
}

#[derive(Debug, Parser)]
struct PrArgs {
    #[arg(long, default_value = "main")]
    base: String,
    #[arg(long)]
    warehouse: Option<String>,
    #[arg(long)]
    dialect: Option<String>,
    #[arg(long, value_enum)]
    format: Option<FormatArg>,
    #[arg(long)]
    manifest: Option<PathBuf>,
    #[arg(long)]
    fail_on: Option<String>,
    #[arg(long)]
    min_confidence: Option<String>,
    #[arg(long)]
    baseline: Option<PathBuf>,
    #[arg(long)]
    cost: bool,
    #[arg(long)]
    fail_on_cost_delta: Option<f64>,
    #[arg(long)]
    analysis_policy: Option<String>,
    #[command(flatten)]
    policy: PolicyArgs,
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

#[derive(Debug, Parser)]
struct RulesArgs {
    #[arg(long, value_enum)]
    format: Option<FormatArg>,
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
            let config = match config_from_scan_args(args).context("configuration error") {
                Ok(config) => config,
                Err(err) => return configuration_error(err),
            };
            let result = scan(&config)?;
            print!("{}", render(&result, config.format)?);
            Ok(
                if result.should_fail(
                    config.fail_on,
                    config.min_confidence,
                    fail_on_monthly_delta(&config),
                    fail_on_monthly_delta_gb(&config),
                ) {
                    1
                } else {
                    0
                },
            )
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
            let config = match config_from_pr_args(args).context("configuration error") {
                Ok(config) => config,
                Err(err) => return configuration_error(err),
            };
            let result = scan(&config)?;
            print!("{}", render(&result, config.format)?);
            Ok(
                if result.should_fail(
                    config.fail_on,
                    config.min_confidence,
                    fail_on_monthly_delta(&config),
                    fail_on_monthly_delta_gb(&config),
                ) {
                    1
                } else {
                    0
                },
            )
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
        Command::Baseline(args) => match args.command {
            BaselineCommand::MigrateV1 {
                input,
                output,
                warehouse,
                manifest,
                paths,
            } => {
                let mut config = base_config()?;
                if !paths.is_empty() {
                    config.paths = paths;
                }
                ScanRuntimeOverrides {
                    warehouse,
                    manifest_path: manifest,
                    ..ScanRuntimeOverrides::default()
                }
                .apply_to(&mut config)?;
                config.baseline_path = None;
                config.write_baseline_path = None;
                config.fail_on = None;
                validate_scan_config(&config)?;
                let result = scan(&config)?;
                let legacy = costguard_core::load_legacy_baseline_v1(&input)?;
                let migrated = costguard_core::migrate_legacy_baseline_v1(
                    &legacy,
                    &result.diagnostics,
                    config.platform,
                    Some(result.policy.digest),
                )?;
                costguard_core::write_finding_baseline(&output, &migrated)?;
                println!(
                    "migrated {} of {} legacy findings to {}",
                    migrated.findings.len(),
                    legacy.findings.len(),
                    output.display()
                );
                Ok(0)
            }
            BaselineCommand::IdentityMapV2 {
                output,
                warehouse,
                manifest,
                paths,
                policy,
            } => {
                let config = match config_from_identity_map_args(IdentityMapArgs {
                    paths,
                    warehouse,
                    manifest,
                    policy,
                })
                .context("configuration error")
                {
                    Ok(config) => config,
                    Err(err) => return configuration_error(err),
                };
                let (result, legacy_aliases) = scan_for_identity_map(&config)?;
                let identity_map = generate_identity_map(
                    &config,
                    legacy_aliases,
                    Some(result.policy.digest.clone()),
                )?;
                write_json(&output, &identity_map)?;
                println!(
                    "wrote {} identity map entries to {}",
                    identity_map.entries.len(),
                    output.display()
                );
                Ok(0)
            }
            BaselineCommand::MigrateV2 {
                input,
                map,
                output,
                policy,
                trust_store,
            } => {
                let baseline = load_baseline_v2(&input).context("baseline migration failed")?;
                let identity_map = load_identity_map(&map).context("baseline migration failed")?;
                let policy_digest = match resolve_optional_policy_digest(policy, trust_store) {
                    Ok(digest) => digest,
                    Err(err) => return migration_error(err),
                };
                let migrated = match migrate_baseline_v2(&baseline, &identity_map, policy_digest) {
                    Ok(migrated) => migrated,
                    Err(err) => return migration_error(err.context("baseline migration failed")),
                };
                costguard_core::write_finding_baseline(&output, &migrated)?;
                println!(
                    "migrated {} baseline findings to {}",
                    migrated.findings.len(),
                    output.display()
                );
                Ok(0)
            }
        },
        Command::Policy(args) => run_policy_command(args.command),
    }
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
        PolicyCommand::MigrateV1 {
            input,
            map,
            output,
            version,
            issued_at,
            expires_at,
            trust_store: _,
        } => {
            let policy_v1 = load_policy_v1_json(&input).context("policy migration failed")?;
            let identity_map = load_identity_map(&map).context("policy migration failed")?;
            let expires_at = match expires_at {
                Some(value) => value,
                None => policy_v1.expires_at.clone(),
            };
            let migrated =
                match migrate_policy_v1(policy_v1, &identity_map, version, issued_at, expires_at) {
                    Ok(migrated) => migrated,
                    Err(err) => return migration_error(err.context("policy migration failed")),
                };
            write_json(&output, &migrated)?;
            println!(
                "migrated policy {} to {} ({})",
                migrated.id,
                output.display(),
                policy_digest(&migrated)?
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
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
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

fn migration_error(err: anyhow::Error) -> Result<u8> {
    eprintln!("error: {err:#}");
    Ok(2)
}

fn resolve_optional_policy_digest(
    bundle: Option<PathBuf>,
    trust_store: Option<PathBuf>,
) -> Result<Option<String>> {
    match (bundle, trust_store) {
        (None, None) => Ok(None),
        (Some(bundle), Some(trust_store)) => {
            let root = std::env::current_dir().context("failed to resolve current directory")?;
            let resolve_path = |path: PathBuf| {
                if path.is_absolute() {
                    path
                } else {
                    root.join(path)
                }
            };
            let signed = read_signed_policy(&resolve_path(bundle))?;
            let trust = read_trust_store(&resolve_path(trust_store))?;
            let policy = verify_policy(&signed, &trust, chrono::Utc::now())?;
            Ok(Some(policy_digest(&policy)?))
        }
        _ => anyhow::bail!("both --policy and --trust-store are required together"),
    }
}

struct IdentityMapArgs {
    paths: Vec<PathBuf>,
    warehouse: Option<String>,
    manifest: Option<PathBuf>,
    policy: PolicyArgs,
}

fn config_from_identity_map_args(args: IdentityMapArgs) -> Result<ScanConfig> {
    let mut config = base_config()?;
    if !args.paths.is_empty() {
        config.paths = args.paths;
    }
    ScanRuntimeOverrides {
        warehouse: args.warehouse,
        manifest_path: args.manifest,
        policy_bundle_path: args.policy.bundle,
        trust_store_path: args.policy.trust_store,
        policy_organization: args.policy.organization,
        policy_team: args.policy.team,
        policy_repository: args.policy.repository,
        ..ScanRuntimeOverrides::default()
    }
    .apply_to(&mut config)?;
    config.baseline_path = None;
    config.write_baseline_path = None;
    config.fail_on = None;
    validate_scan_config(&config)?;
    Ok(config)
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

fn config_from_scan_args(args: ScanArgs) -> Result<ScanConfig> {
    let mut config = base_config()?;
    if !args.paths.is_empty() {
        config.paths = args.paths;
    }
    ScanRuntimeOverrides {
        warehouse: args.warehouse,
        dialect: args.dialect,
        format: args.format.map(Into::into),
        manifest_path: args.manifest,
        fail_on: args.fail_on,
        min_confidence: args.min_confidence,
        baseline_path: args.baseline,
        write_baseline_path: args.write_baseline,
        cost: args.cost,
        fail_on_cost_delta: args.fail_on_cost_delta,
        analysis_policy: args.analysis_policy,
        policy_bundle_path: args.policy.bundle,
        trust_store_path: args.policy.trust_store,
        policy_organization: args.policy.organization,
        policy_team: args.policy.team,
        policy_repository: args.policy.repository,
    }
    .apply_to(&mut config)?;
    validate_scan_config(&config)?;
    Ok(config)
}

fn config_from_explain_args(args: &ExplainArgs) -> Result<ScanConfig> {
    let mut config = base_config()?;
    ScanRuntimeOverrides {
        warehouse: args.warehouse.clone(),
        dialect: args.dialect.clone(),
        format: args.format.map(Into::into),
        manifest_path: args.manifest.clone(),
        cost: args.cost,
        analysis_policy: args.analysis_policy.clone(),
        policy_bundle_path: args.policy.bundle.clone(),
        trust_store_path: args.policy.trust_store.clone(),
        policy_organization: args.policy.organization.clone(),
        policy_team: args.policy.team.clone(),
        policy_repository: args.policy.repository.clone(),
        ..ScanRuntimeOverrides::default()
    }
    .apply_to(&mut config)?;
    validate_scan_config(&config)?;
    Ok(config)
}

fn config_from_cost_args(args: CostReportArgs) -> Result<ScanConfig> {
    let mut config = base_config()?;
    if !args.paths.is_empty() {
        config.paths = args.paths;
    }
    ScanRuntimeOverrides {
        warehouse: args.warehouse,
        dialect: args.dialect,
        format: args.format.map(Into::into),
        manifest_path: args.manifest,
        cost: true,
        analysis_policy: args.analysis_policy,
        policy_bundle_path: args.policy.bundle,
        trust_store_path: args.policy.trust_store,
        policy_organization: args.policy.organization,
        policy_team: args.policy.team,
        policy_repository: args.policy.repository,
        ..ScanRuntimeOverrides::default()
    }
    .apply_to(&mut config)?;
    validate_scan_config(&config)?;
    Ok(config)
}

fn config_from_pr_args(args: PrArgs) -> Result<ScanConfig> {
    let mut config = base_config()?;
    config.changed_only = true;
    config.base_branch = Some(args.base);
    ScanRuntimeOverrides {
        warehouse: args.warehouse,
        dialect: args.dialect,
        format: args.format.map(Into::into),
        manifest_path: args.manifest,
        fail_on: args.fail_on,
        min_confidence: args.min_confidence,
        baseline_path: args.baseline,
        cost: args.cost,
        fail_on_cost_delta: args.fail_on_cost_delta,
        analysis_policy: args.analysis_policy,
        policy_bundle_path: args.policy.bundle,
        trust_store_path: args.policy.trust_store,
        policy_organization: args.policy.organization,
        policy_team: args.policy.team,
        policy_repository: args.policy.repository,
        ..ScanRuntimeOverrides::default()
    }
    .apply_to(&mut config)?;
    validate_scan_config(&config)?;
    Ok(config)
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
