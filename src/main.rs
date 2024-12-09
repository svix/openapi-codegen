use std::{collections::BTreeMap, path::Path};

use aide::openapi::{self, OpenApi};
use anyhow::Context as _;
use clap::{Parser, Subcommand};
use fs_err as fs;
use tempfile::TempDir;

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
    tracing_subscriber::fmt::init();

    let args = CliArgs::parse();
    let Command::Generate { input_file } = args.command;

    let spec = fs::read_to_string(&input_file)?;
    let spec: OpenApi = serde_json::from_str(&spec).context("failed to parse OpenAPI spec")?;

    let output_dir = TempDir::new().context("failed to create tempdir")?;

    let empty_components = openapi::Components::default();
    let components = spec.components.as_ref().unwrap_or(&empty_components);

    if let Some(paths) = spec.paths {
        let api = Api::new(paths, components).unwrap();
        dbg!(api).write_rust_stuff(&output_dir).unwrap();
    }

    Ok(())
}

#[derive(Debug)]
struct Api {
    resources: BTreeMap<String, Resource>,
}

#[derive(Debug, Default)]
struct Resource {
    operations: Vec<Operation>,
    // TODO: subresources?
}

#[derive(Debug)]
struct Operation {
    /// The name to use for the operation in code.
    name: String,
    /// The HTTP method.
    ///
    /// Encoded as "get", "post" or such because that's what aide's PathItem iterator gives us.
    method: String,
    /// The operation's endpoint path.
    path: String,
}

impl Api {
    fn new(paths: openapi::Paths, components: &openapi::Components) -> anyhow::Result<Self> {
        let mut resources = BTreeMap::new();

        for (path, pi) in paths {
            let path_item = pi
                .into_item()
                .context("$ref paths are currently not supported")?;
            for (method, op) in path_item {
                let Some(op_id) = &op.operation_id else {
                    // ignore operations without an operationId
                    continue;
                };
                let op_id_parts: Vec<_> = op_id.split(".").collect();
                let Ok([version, res_name, op_name]): Result<[_; 3], _> = op_id_parts.try_into()
                else {
                    tracing::info!(op_id, "skipping operation whose ID does not have two dots");
                    continue;
                };
                if version != "v1" {
                    tracing::warn!(op_id, "found operation whose ID does not begin with v1");
                    continue;
                }

                let resource = resources
                    .entry(res_name.to_owned())
                    .or_insert_with(Resource::default);
                resource.operations.push(Operation {
                    name: op_name.to_owned(),
                    method: method.to_owned(),
                    path: path.clone(),
                });
            }
        }

        Ok(Self { resources })
    }

    fn write_rust_stuff(self, output_dir: impl AsRef<Path>) -> anyhow::Result<()> {
        let _api_dir = output_dir.as_ref().join("api");

        Ok(())
    }
}
