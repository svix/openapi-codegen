use std::{
    collections::BTreeSet,
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

        /// The specified operations for --include-mode=specified
        ///
        /// This expects the operation ID, for example v1.message.create
        #[clap(long)]
        specified_operations: Vec<String>,
    },
}

// Boolean flags for generate command, separate struct to simplify passing them around.
#[derive(Clone, Copy, clap::Args)]
struct GenerateFlags {
    /// Disable automatic postprocessing of the output (formatting and automatic style fixes).
    #[clap(long)]
    no_postprocess: bool,

    /// Which operations to include
    #[clap(long, value_enum, default_value_t=IncludeMode::OnlyPublic)]
    include_mode: IncludeMode,

    /// Write api.ron and types.ron files, as a debugging aid.
    #[clap(long)]
    debug: bool,
}

#[derive(Copy, Clone, clap::ValueEnum)]
#[clap(rename_all = "kebab-case")]
enum IncludeMode {
    /// Only public options
    OnlyPublic,
    /// Both public operations and operations marked with `x-hidden`
    PublicAndHidden,
    /// Only operations marked with `x-hidden`
    OnlyHidden,
    /// Only include operations specified in `--specified-operations`
    Specified,
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_writer(io::stderr).init();

    let args = CliArgs::parse();
    let Command::Generate {
        template,
        input_file,
        output_dir,
        flags,
        specified_operations,
    } = args.command;
    let specified_operations = BTreeSet::from_iter(specified_operations);

    let spec = fs::read_to_string(&input_file)?;

    let spec: OpenApi = serde_json::from_str(&spec).context("failed to parse OpenAPI spec")?;

    match &output_dir {
        Some(path) => {
            analyze_and_generate(spec, template.into(), path, flags, specified_operations)?;
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
            analyze_and_generate(spec, template.into(), path, flags, specified_operations)?;
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
    specified_operations: BTreeSet<String>,
) -> anyhow::Result<()> {
    let webhooks = get_webhooks(&spec);
    let mut components = spec.components.unwrap_or_default();
    if let Some(paths) = spec.paths {
        let api = Api::new(
            paths,
            &components.schemas,
            flags.include_mode,
            specified_operations,
        )
        .unwrap();
        let types = api.types(&mut components.schemas, webhooks);

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

fn get_webhooks(spec: &OpenApi) -> Vec<String> {
    let empty_obj = serde_json::json!({});
    let empty_obj = empty_obj.as_object().unwrap();
    let mut referenced_components = std::collections::BTreeSet::<String>::new();
    if let Some(webhooks) = spec.extensions.get("x-webhooks") {
        for req in webhooks.as_object().unwrap_or(empty_obj).values() {
            for method in req.as_object().unwrap_or(empty_obj).values() {
                if let Some(schema_ref) = method
                    .get("requestBody")
                    .and_then(|v| v.get("content"))
                    .and_then(|v| v.get("application/json"))
                    .and_then(|v| v.get("schema"))
                    .and_then(|v| v.get("$ref"))
                    .and_then(|v| v.as_str())
                {
                    if let Some(schema_name) = schema_ref.split('/').next_back() {
                        referenced_components.insert(schema_name.to_string());
                    }
                }
            }
        }
    }
    referenced_components.into_iter().collect::<Vec<String>>()
}
