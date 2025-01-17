use std::{
    io::{self, BufWriter, Write as _},
    path::PathBuf,
};

use aide::openapi::OpenApi;
use anyhow::Context as _;
use camino::Utf8Path;
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

        /// Generate code for deprecated operations, too.
        #[clap(long)]
        with_deprecated: bool,

        /// Override the output dir (useful if you want to have the generator rewrite the file in the same location)
        #[clap(long)]
        override_output_dir: Option<PathBuf>,
    },
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_writer(io::stderr).init();

    let args = CliArgs::parse();
    let Command::Generate {
        template,
        input_file,
        no_postprocess,
        with_deprecated,
        override_output_dir,
    } = args.command;

    let spec = fs::read_to_string(&input_file)?;
    let spec: OpenApi = serde_json::from_str(&spec).context("failed to parse OpenAPI spec")?;

    let output_dir_root = PathBuf::from("out");
    if !output_dir_root.exists() {
        fs::create_dir(&output_dir_root).context("failed to create out dir")?;
    }
    let tmp_output_dir = TempDir::new_in(&output_dir_root).context("failed to create tempdir")?;

    let utf8_output_dir: &Utf8Path = {
        if override_output_dir.is_some() {
            override_output_dir.as_ref().unwrap().as_path()
        } else {
            tmp_output_dir.path()
        }
    }
    .try_into()
    .context("non-UTF8 tempdir path")?;

    let mut components = spec.components.unwrap_or_default();

    if let Some(paths) = spec.paths {
        let api = Api::new(paths, with_deprecated, &components.schemas).unwrap();
        {
            let mut api_file = BufWriter::new(File::create("api.ron")?);
            writeln!(api_file, "{api:#?}")?;
        }

        let types = api.types(&mut components.schemas);
        {
            let mut types_file = BufWriter::new(File::create("types.ron")?);
            writeln!(types_file, "{types:#?}")?;
        }

        generate(api, types, template, utf8_output_dir, no_postprocess)?;
    }

    match &override_output_dir {
        Some(p) => println!("done! output written to {}", p.display()),
        None => println!(
            "done! output written to {}",
            // if everything has succeeded (and override_output_dir is None), keep the tempdir for further use
            tmp_output_dir.into_path().display()
        ),
    };

    Ok(())
}
