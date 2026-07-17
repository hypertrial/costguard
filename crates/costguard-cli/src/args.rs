use super::Command;
use clap::Parser;

#[derive(Debug, Parser)]
#[command(name = "costguard")]
#[command(about = "Local, dbt-aware cost regression guardrail for git workflows")]
#[command(version, propagate_version = true)]
pub(super) struct Cli {
    #[command(subcommand)]
    pub(super) command: Command,
}
