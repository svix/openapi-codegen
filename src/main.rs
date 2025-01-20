use std::{
    io::{self, BufWriter, Write as _},
    path::PathBuf,
};

use aide::openapi::OpenApi;
use anyhow::Context as _;
use camino::{Utf8Path, Utf8PathBuf};
use clap::{Parser, Subcommand};
use fs_err::{self as fs, File};
use tempfile::TempDir;

mod api;
mod generator;
mod template;
mod types;
mod util;

use self::{api::Api, generator::generate};

#[derive(Parser)]
struct CliArgs {
    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, Subcommand)]
enum Command {
    /// Generate code from an OpenAPI spec.
    Generate {
        /// The template to use (`.jinja` extension can be omitted).
        #[clap(short, long)]
        template: String,

        /// Path to the input file.
        #[clap(short, long)]
        input_file: String,

        /// Disable automatic postprocessing of the output (formatting and automatic style fixes).
        #[clap(long)]
        no_postprocess: bool,

        /// Set the output dir
        #[clap(long)]
        output_dir: Option<Utf8PathBuf>,
    },
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_writer(io::stderr).init();

    let args = CliArgs::parse();
    let Command::Generate {
        template,
        input_file,
        no_postprocess,
        output_dir,
    } = args.command;

    let spec = fs::read_to_string(&input_file)?;
    let spec: OpenApi = serde_json::from_str(&spec).context("failed to parse OpenAPI spec")?;

    match &output_dir {
        Some(path) => {
            analyze_and_generate(spec, template, path, no_postprocess)?;
            println!("done! output written to {path}");
        }
        None => {
            let output_dir_root = PathBuf::from("out");
            if !output_dir_root.exists() {
                fs::create_dir(&output_dir_root).context("failed to create out dir")?;
            }
            let tmp_output_dir =
                TempDir::new_in(&output_dir_root).context("failed to create tempdir")?;

            // create if doesn't exist
            let path = tmp_output_dir
                .path()
                .try_into()
                .context("non-UTF8 tempdir path")?;
            analyze_and_generate(spec, template, path, no_postprocess)?;
            // Persist the TempDir if everything was successful
            let path = tmp_output_dir.into_path();

            println!("done! output written to {}", path.display());
        }
    };

    Ok(())
}

fn analyze_and_generate(
    spec: OpenApi,
    template: String,
    path: &Utf8Path,
    no_postprocess: bool,
) -> anyhow::Result<()> {
    let mut components = spec.components.unwrap_or_default();

    if let Some(paths) = spec.paths {
        let api = Api::new(paths, &components.schemas).unwrap();
        {
            let mut api_file = BufWriter::new(File::create("api.ron")?);
            writeln!(api_file, "{api:#?}")?;
        }

        let types = api.types(&mut components.schemas);
        {
            let mut types_file = BufWriter::new(File::create("types.ron")?);
            writeln!(types_file, "{types:#?}")?;
        }

        generate(api, types, template, path, no_postprocess)?;
    }
    Ok(())
}
