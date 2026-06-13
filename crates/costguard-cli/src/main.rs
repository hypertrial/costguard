use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use costguard_core::{
    apply_file_config, explain, load_config, rules, scan, validate_scan_config, OutputFormat,
    ScanConfig, ScanRuntimeOverrides,
};
use costguard_output::{render, render_rules};
use std::path::PathBuf;
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
    }
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
