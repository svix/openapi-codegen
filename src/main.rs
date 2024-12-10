#![allow(dead_code)]

use std::{
    collections::BTreeMap,
    io::{self, BufWriter, Write as _},
};

use aide::openapi::OpenApi;
use anyhow::Context as _;
use clap::{Parser, Subcommand};
use fs_err::{self as fs, File};
use schemars::schema::SchemaObject;
use tempfile::TempDir;

mod api;

use self::api::Api;

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
    tracing_subscriber::fmt().with_writer(io::stderr).init();

    let args = CliArgs::parse();
    let Command::Generate { input_file } = args.command;

    let spec = fs::read_to_string(&input_file)?;
    let spec: OpenApi = serde_json::from_str(&spec).context("failed to parse OpenAPI spec")?;

    let output_dir = TempDir::new().context("failed to create tempdir")?;

    let mut components = spec.components.unwrap_or_default();

    if let Some(paths) = spec.paths {
        let api = Api::new(paths).unwrap();
        {
            let mut api_file = BufWriter::new(File::create("api.ron")?);
            writeln!(api_file, "{api:#?}")?;
        }

        {
            let types = api.types(&mut components.schemas);
            let mut types_file = BufWriter::new(File::create("types.ron")?);
            writeln!(types_file, "{types:#?}")?;
        }

        api.write_rust_stuff(&output_dir)?;
    }

    Ok(())
}

/// Named types referenced by the [`Api`].
///
/// Intermediate representation of (some) `components` from the spec.
#[derive(Debug)]
struct Types(BTreeMap<String, SchemaObject>);
