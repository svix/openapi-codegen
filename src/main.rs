use std::{collections::BTreeMap, io, path::Path};

use aide::openapi::{self, OpenApi, ReferenceOr};
use anyhow::Context as _;
use clap::{Parser, Subcommand};
use fs_err as fs;
use schemars::schema::SchemaObject;
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
    tracing_subscriber::fmt().with_writer(io::stderr).init();

    let args = CliArgs::parse();
    let Command::Generate { input_file } = args.command;

    let spec = fs::read_to_string(&input_file)?;
    let spec: OpenApi = serde_json::from_str(&spec).context("failed to parse OpenAPI spec")?;

    let output_dir = TempDir::new().context("failed to create tempdir")?;

    let empty_components = openapi::Components::default();
    let components = spec.components.as_ref().unwrap_or(&empty_components);

    if let Some(paths) = spec.paths {
        let api = Api::new(paths, components).unwrap();
        println!("{api:#?}");
        api.write_rust_stuff(&output_dir).unwrap();
    }

    Ok(())
}

/// Named types referenced by the [`Api`].
///
/// Intermediate representation of (some) `components` from the spec.
#[derive(Debug)]
struct Types(BTreeMap<String, SchemaObject>);

/// The API we generate a client for.
///
/// Intermediate representation of `paths` from the spec.
#[derive(Debug)]
struct Api {
    resources: BTreeMap<String, Resource>,
}

/// A named group of [`Operation`]s.
#[derive(Debug, Default)]
struct Resource {
    operations: Vec<Operation>,
    // TODO: subresources?
}

/// A named HTTP endpoint.
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
    /// Path parameters.
    path_params: Vec<openapi::SchemaObject>,
    /// Name of the request body type, if any.
    request_body: Option<openapi::RequestBody>,
}

impl Operation {
    #[tracing::instrument(name = "operation_from_openapi", skip(op), fields(op_id))]
    fn from_openapi(path: &str, method: &str, op: openapi::Operation) -> Option<(String, Self)> {
        let Some(op_id) = &op.operation_id else {
            // ignore operations without an operationId
            return None;
        };
        let op_id_parts: Vec<_> = op_id.split(".").collect();
        let Ok([version, res_name, op_name]): Result<[_; 3], _> = op_id_parts.try_into() else {
            tracing::debug!(op_id, "skipping operation whose ID does not have two dots");
            return None;
        };
        if version != "v1" {
            tracing::warn!(op_id, "found operation whose ID does not begin with v1");
            return None;
        }

        let mut path_params = Vec::new();

        for param in op.parameters {
            match param {
                ReferenceOr::Reference { .. } => {
                    tracing::warn!("$ref parameters are not currently supported");
                }
                ReferenceOr::Item(openapi::Parameter::Path {
                    parameter_data,
                    style: openapi::PathStyle::Simple,
                }) => {
                    assert!(parameter_data.required, "no optional path params");
                    let openapi::ParameterSchemaOrContent::Schema(schema_object) =
                        parameter_data.format
                    else {
                        tracing::warn!("found unexpected 'content' parameter data format");
                        return None;
                    };
                    path_params.push(schema_object);
                }
                // used for idempotency-key
                ReferenceOr::Item(openapi::Parameter::Header { .. }) => {
                    // TODO
                    return None;
                }
                // used for list parameters
                ReferenceOr::Item(openapi::Parameter::Query { .. }) => {
                    // TODO
                    return None;
                }
                ReferenceOr::Item(parameter) => {
                    tracing::warn!(
                        ?parameter,
                        "this kind of parameter is not currently supported"
                    );
                    return None;
                }
            }
        }

        let request_body = op.request_body.and_then(|b| match b {
            ReferenceOr::Item(req_body) => Some(req_body),
            ReferenceOr::Reference { .. } => {
                tracing::warn!("$ref request bodies are not currently supported");
                None
            }
        });

        let op = Operation {
            name: op_name.to_owned(),
            method: method.to_owned(),
            path: path.to_owned(),
            path_params,
            request_body,
        };
        Some((res_name.to_owned(), op))
    }
}

impl Api {
    fn new(paths: openapi::Paths, components: &openapi::Components) -> anyhow::Result<Self> {
        let mut resources = BTreeMap::new();

        for (path, pi) in paths {
            let path_item = pi
                .into_item()
                .context("$ref paths are currently not supported")?;

            if !path_item.parameters.is_empty() {
                tracing::info!("parameters at the path item level are not currently supported");
                continue;
            }

            for (method, op) in path_item {
                if let Some((res_name, op)) = Operation::from_openapi(&path, method, op) {
                    let resource = resources
                        .entry(res_name.to_owned())
                        .or_insert_with(Resource::default);
                    resource.operations.push(op);
                }
            }
        }

        Ok(Self { resources })
    }

    fn write_rust_stuff(self, output_dir: impl AsRef<Path>) -> anyhow::Result<()> {
        let _api_dir = output_dir.as_ref().join("api");

        Ok(())
    }
}
