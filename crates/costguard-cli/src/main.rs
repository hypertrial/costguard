use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use costguard_core::{
    apply_file_config, explain, load_config, rules, scan, validate_scan_config, OutputFormat,
    ScanConfig, ScanRuntimeOverrides,
};
use costguard_output::{render, render_rules};
use costguard_policy::{
    canonical_json, compile_toml, generate_key, policy_digest, read_signed_policy,
    read_trust_store, resolve_policy, sign_policy, verify_policy, PolicyDocumentV1,
    ResolutionContext, TrustStoreV1, TrustedKeyV1,
};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Debug, Parser)]
#[command(name = "costguard")]
#[command(about = "Static cost and performance guardrails for dbt and warehouse SQL")]
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
        Command::Cost(args) => {
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
        },
        Command::Policy(args) => run_policy_command(args.command),
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
            let policy: PolicyDocumentV1 = read_json(&policy, "compiled policy")?;
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

fn config_from_cost_args(args: CostArgs) -> Result<ScanConfig> {
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
