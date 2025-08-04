use std::{
    collections::BTreeSet,
    io,
    path::{Path, PathBuf},
};

use aide::openapi::OpenApi;
use anyhow::{bail, Context as _};
use camino::Utf8PathBuf;
use clap::{Parser, Subcommand};
use fs_err::{self as fs};
use tempfile::TempDir;

mod api;
mod generator;
mod postprocessing;
mod template;

use self::{api::Api, generator::generate};

#[derive(Parser)]
struct CliArgs {
    /// Which operations to include.
    #[arg(global = true, long, value_enum, default_value_t = IncludeMode::OnlyPublic)]
    include_mode: IncludeMode,

    /// Ignore a specified operation id
    #[arg(global = true, short, long = "exclude-op-id")]
    excluded_operations: Vec<String>,

    /// Only include specified operations
    ///
    /// This option only works with `--include-mode=only-specified`.
    ///
    /// Use this option, to run the codegen with a limited set of operations.
    /// Op webhook models will be excluded from the generation
    #[arg(global = true, long = "include-op-id")]
    specified_operations: Vec<String>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, Subcommand)]
enum Command {
    /// Generate code from an OpenAPI spec.
    Generate {
        /// Path to a template file to use (`.jinja` extension can be omitted).
        #[arg(short, long)]
        template: Utf8PathBuf,

        /// Path to the input file.
        #[arg(short, long)]
        input_file: String,

        /// Path to the output directory.
        #[arg(short, long)]
        output_dir: Option<Utf8PathBuf>,

        /// Disable automatic postprocessing of the output (formatting and automatic style fixes).
        #[arg(long)]
        no_postprocess: bool,
    },
    /// Generate api.ron and types.ron files, for debugging.
    Debug {
        /// Path to the input file.
        #[arg(short, long)]
        input_file: String,
    },
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
    /// Only operations that were specified in `--include-op-id`
    OnlySpecified,
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_writer(io::stderr).init();

    let args = CliArgs::parse();
    let input_file = match &args.command {
        Command::Generate { input_file, .. } => input_file,
        Command::Debug { input_file } => input_file,
    };

    let excluded_operations = BTreeSet::from_iter(args.excluded_operations);
    let specified_operations = BTreeSet::from_iter(args.specified_operations);

    let input_file = Path::new(input_file);
    let input_file_ext = input_file
        .extension()
        .context("input file must have a file extension")?;
    let input_file_contents = fs::read_to_string(input_file)?;

    let api = if input_file_ext == "json" {
        let spec: OpenApi =
            serde_json::from_str(&input_file_contents).context("failed to parse OpenAPI spec")?;

        let webhooks = get_webhooks(&spec);
        Api::new(
            spec.paths.context("found no endpoints in input spec")?,
            &mut spec.components.unwrap_or_default(),
            &webhooks,
            args.include_mode,
            excluded_operations,
            specified_operations,
        )?
    } else if input_file_ext == "ron" {
        ron::from_str(&input_file_contents)?
    } else {
        bail!("input file extension must be .json or .ron");
    };

    match args.command {
        Command::Generate {
            template,
            output_dir,
            no_postprocess,
            ..
        } => {
            match &output_dir {
                Some(path) => {
                    generate(api, template.into(), path, no_postprocess)?;
                    println!("done! output written to {path}");
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

                    let output_dir =
                        TempDir::with_prefix_in(prefix.to_owned() + ".", output_dir_root)
                            .context("failed to create tempdir")?;

                    let path = output_dir
                        .path()
                        .try_into()
                        .context("non-UTF8 tempdir path")?;

                    generate(api, template.into(), path, no_postprocess)?;
                    println!("done! output written to {path}");

                    // Persist the TempDir if everything was successful
                    _ = output_dir.keep();
                }
            }
        }
        Command::Debug { .. } => {
            let serialized = ron::ser::to_string_pretty(
                &api,
                ron::ser::PrettyConfig::new()
                    .extensions(ron::extensions::Extensions::IMPLICIT_SOME),
            )?;
            fs::write("debug.ron", serialized)?;
        }
    }

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
