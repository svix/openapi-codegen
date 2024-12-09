use aide::openapi::OpenApi;
use anyhow::Context as _;
use clap::{Parser, Subcommand};
use fs_err as fs;

#[derive(Parser)]
struct CliArgs {
    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, Subcommand)]
enum Command {
    Generate { input_file: String },
}

fn main() -> anyhow::Result<()> {
    let args = CliArgs::parse();
    let Command::Generate { input_file } = args.command;

    let spec = fs::read_to_string(&input_file)?;
    let spec: OpenApi = serde_json::from_str(&spec).context("failed to parse OpenAPI spec")?;

    dbg!(spec.info);

    Ok(())
}
