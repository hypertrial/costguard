use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use costguard_core::{
    apply_file_config, explain, load_config, rules, scan, OutputFormat, Platform, ScanConfig,
};
use costguard_diagnostics::Severity;
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
                if result.should_fail(config.fail_on, config.min_confidence) {
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
                if result.should_fail(config.fail_on, config.min_confidence) {
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
                if result.should_fail(config.fail_on, config.min_confidence) {
                    1
                } else {
                    0
                },
            )
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
    apply_common_flags(
        &mut config,
        args.warehouse,
        args.dialect,
        args.format,
        args.manifest,
        args.fail_on,
        args.min_confidence,
        args.baseline,
        args.write_baseline,
    )?;
    validate_config(&config)?;
    Ok(config)
}

fn config_from_explain_args(args: &ExplainArgs) -> Result<ScanConfig> {
    let mut config = base_config()?;
    apply_common_flags(
        &mut config,
        args.warehouse.clone(),
        args.dialect.clone(),
        args.format,
        args.manifest.clone(),
        None,
        None,
        None,
        None,
    )?;
    validate_config(&config)?;
    Ok(config)
}

fn config_from_pr_args(args: PrArgs) -> Result<ScanConfig> {
    let mut config = base_config()?;
    config.changed_only = true;
    config.base_branch = Some(args.base);
    apply_common_flags(
        &mut config,
        args.warehouse,
        args.dialect,
        args.format,
        args.manifest,
        args.fail_on,
        args.min_confidence,
        args.baseline,
        None,
    )?;
    validate_config(&config)?;
    Ok(config)
}

#[allow(clippy::too_many_arguments)]
fn apply_common_flags(
    config: &mut ScanConfig,
    warehouse: Option<String>,
    dialect: Option<String>,
    format: Option<FormatArg>,
    manifest: Option<PathBuf>,
    fail_on: Option<String>,
    min_confidence: Option<String>,
    baseline: Option<PathBuf>,
    write_baseline: Option<PathBuf>,
) -> Result<()> {
    if let Some(warehouse) = warehouse {
        config.platform = warehouse.parse::<Platform>().map_err(anyhow::Error::msg)?;
    } else if let Some(dialect) = dialect {
        config.platform = dialect.parse::<Platform>().map_err(anyhow::Error::msg)?;
    }
    if let Some(format) = format {
        config.format = format.into();
    }
    if let Some(manifest) = manifest {
        config.manifest_path = Some(manifest);
    }
    if let Some(fail_on) = fail_on {
        config.fail_on = Some(fail_on.parse::<Severity>().map_err(anyhow::Error::msg)?);
    }
    if let Some(min_confidence) = min_confidence {
        config.min_confidence = Some(min_confidence.parse().map_err(anyhow::Error::msg)?);
    }
    if let Some(baseline) = baseline {
        config.baseline_path = Some(baseline);
    }
    if let Some(write_baseline) = write_baseline {
        config.write_baseline_path = Some(write_baseline);
    }
    Ok(())
}

fn validate_config(config: &ScanConfig) -> Result<()> {
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
    }
    Ok(())
}
