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
mod postprocessing;
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
        /// Path to a template file to use (`.jinja` extension can be omitted).
        #[clap(short, long)]
        template: Utf8PathBuf,

        /// Path to the input file.
        #[clap(short, long)]
        input_file: String,

        /// Path to the output directory.
        #[clap(long)]
        output_dir: Option<Utf8PathBuf>,

        #[clap(flatten)]
        flags: GenerateFlags,
    },
}

// Boolean flags for generate command, separate struct to simplify passing them around.
#[derive(Clone, Copy, clap::Args)]
struct GenerateFlags {
    /// Disable automatic postprocessing of the output (formatting and automatic style fixes).
    #[clap(long)]
    no_postprocess: bool,

    /// Include operations in the output that are marked `"x-hidden": true`.
    #[clap(long)]
    include_hidden: bool,

    /// Write api.ron and types.ron files, as a debugging aid.
    #[clap(long)]
    debug: bool,

    /// Write `.codegen.json` file
    #[clap(long)]
    write_codegen_metadata: bool,

    /// When generating a file the parent directories are crated if they don't exist
    #[clap(long)]
    create_file_parents: bool,
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_writer(io::stderr).init();

    let args = CliArgs::parse();
    let Command::Generate {
        template,
        input_file,
        output_dir,
        flags,
    } = args.command;

    let spec = fs::read_to_string(&input_file)?;
    let input_sha256sum = util::sha256sum_string(&spec);

    let spec: OpenApi = serde_json::from_str(&spec).context("failed to parse OpenAPI spec")?;

    match &output_dir {
        Some(path) => {
            analyze_and_generate(spec, template.into(), path, flags)?;
            if flags.write_codegen_metadata {
                write_codegen_metadata(input_sha256sum, path)?;
            }
        }
        None => {
            let output_dir_root = PathBuf::from("out");
            if !output_dir_root.exists() {
                fs::create_dir(&output_dir_root).context("failed to create out dir")?;
            }

            let tpl_file_name = template
                .file_name()
                .context("template must have a file name")?;
            let prefix = tpl_file_name
                .strip_suffix(".jinja")
                .unwrap_or(tpl_file_name);

            let output_dir = TempDir::with_prefix_in(prefix.to_owned() + ".", output_dir_root)
                .context("failed to create tempdir")?;

            let path = output_dir
                .path()
                .try_into()
                .context("non-UTF8 tempdir path")?;
            analyze_and_generate(spec, template.into(), path, flags)?;
            if flags.write_codegen_metadata {
                write_codegen_metadata(input_sha256sum, path)?;
            }
            // Persist the TempDir if everything was successful
            _ = output_dir.into_path();
        }
    }

    Ok(())
}

fn analyze_and_generate(
    spec: OpenApi,
    template: String,
    path: &Utf8Path,
    flags: GenerateFlags,
) -> anyhow::Result<()> {
    let mut components = spec.components.unwrap_or_default();

    if let Some(paths) = spec.paths {
        let api = Api::new(paths, &components.schemas, flags.include_hidden).unwrap();
        let types = api.types(&mut components.schemas);

        if flags.debug {
            let mut api_file = BufWriter::new(File::create("api.ron")?);
            writeln!(api_file, "{api:#?}")?;

            let mut types_file = BufWriter::new(File::create("types.ron")?);
            writeln!(types_file, "{types:#?}")?;
        }

        generate(api, types, template, path, flags)?;
    }

    println!("done! output written to {path}");
    Ok(())
}

fn write_codegen_metadata(input_sha256sum: String, output_dir: &Utf8Path) -> anyhow::Result<()> {
    let metadata_path = output_dir.join(".codegen.json");
    let current_timestamp = chrono::Utc::now().to_rfc3339();
    let codegen_metadata = serde_json::json!({
        "openapi-codegen-version": env!("CARGO_PKG_VERSION"),
        "openapi.json-sha256": input_sha256sum,
        "git-rev": env!("VERGEN_GIT_SHA"),
        "file-generated-at": current_timestamp,
    });
    let encoded_metadata = serde_json::to_vec_pretty(&codegen_metadata)?;
    std::fs::write(metadata_path, &encoded_metadata)?;
    Ok(())
}
