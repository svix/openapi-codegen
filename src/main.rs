use std::io::{self, BufWriter, Write as _};

use aide::openapi::OpenApi;
use anyhow::Context as _;
use clap::{Parser, Subcommand};
use fs_err::{self as fs, File};
use tempfile::TempDir;

mod api;
mod template;
mod types;
mod util;

use self::api::Api;

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

        /// Disable automatic formatting of the output.
        #[clap(long)]
        no_format: bool,

        /// Generate code for deprecated operations, too.
        #[clap(long)]
        with_deprecated: bool,
    },
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_writer(io::stderr).init();

    let args = CliArgs::parse();
    let Command::Generate {
        template,
        input_file,
        no_format,
        with_deprecated,
    } = args.command;

    let spec = fs::read_to_string(&input_file)?;
    let spec: OpenApi = serde_json::from_str(&spec).context("failed to parse OpenAPI spec")?;

    let output_dir = TempDir::new().context("failed to create tempdir")?;

    let mut components = spec.components.unwrap_or_default();

    if let Some(paths) = spec.paths {
        let api = Api::new(paths, with_deprecated).unwrap();
        {
            let mut api_file = BufWriter::new(File::create("api.ron")?);
            writeln!(api_file, "{api:#?}")?;
        }

        {
            let types = api.types(&mut components.schemas);
            let mut types_file = BufWriter::new(File::create("types.ron")?);
            writeln!(types_file, "{types:#?}")?;
        }

        api.generate(&template, &output_dir, no_format)?;
    }

    // if everything has succeeded, keep the tempdir for further use
    let path = output_dir.into_path();
    println!("done! output written to {}", path.display());

    Ok(())
}
