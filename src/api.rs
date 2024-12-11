use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use aide::openapi::{self, ReferenceOr};
use anyhow::{bail, Context as _};
use indexmap::IndexMap;
use schemars::schema::{InstanceType, Schema};

use super::Types;

/// The API we generate a client for.
///
/// Intermediate representation of `paths` from the spec.
#[derive(Debug)]
pub(crate) struct Api {
    resources: BTreeMap<String, Resource>,
}

impl Api {
    pub(crate) fn new(paths: openapi::Paths) -> anyhow::Result<Self> {
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

    fn referenced_components(&self) -> impl Iterator<Item = &str> {
        self.resources
            .values()
            .flat_map(|resource| &resource.operations)
            .filter_map(|operation| operation.request_body_schema_name.as_deref())
    }

    pub(crate) fn types(&self, schemas: &mut IndexMap<String, openapi::SchemaObject>) -> Types {
        let components: BTreeSet<_> = self.referenced_components().collect();
        Types(
            components
                .into_iter()
                .filter_map(|schema_name| {
                    let Some(s) = schemas.swap_remove(schema_name) else {
                        tracing::warn!(schema_name, "schema not found");
                        return None;
                    };
                    match s.json_schema {
                        Schema::Bool(_) => {
                            tracing::warn!("found $ref'erenced bool schema, wat?!");
                            None
                        }
                        Schema::Object(schema_object) => {
                            Some((schema_name.to_owned(), schema_object))
                        }
                    }
                })
                .collect(),
        )
    }

    pub(crate) fn write_rust_stuff(self, output_dir: impl AsRef<Path>) -> anyhow::Result<()> {
        let _api_dir = output_dir.as_ref().join("api");

        Ok(())
    }
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
    ///
    /// Only required string-typed parameters are currently supported.
    path_params: Vec<String>,
    /// Header parameters.
    ///
    /// Only string-typed parameters are currently supported.
    header_params: Vec<HeaderParam>,
    /// Query parameters.
    query_params: Vec<openapi::ParameterData>,
    /// Name of the request body type, if any.
    request_body_schema_name: Option<String>,
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
        let mut query_params = Vec::new();
        let mut header_params = Vec::new();

        for param in op.parameters {
            match param {
                ReferenceOr::Reference { .. } => {
                    tracing::warn!("$ref parameters are not currently supported");
                    return None;
                }
                ReferenceOr::Item(openapi::Parameter::Path {
                    parameter_data,
                    style: openapi::PathStyle::Simple,
                }) => {
                    assert!(parameter_data.required, "no optional path params");
                    if let Err(e) = enforce_string_parameter(&parameter_data) {
                        tracing::warn!("unsupported path parameter: {e}");
                        return None;
                    }

                    path_params.push(parameter_data.name);
                }
                ReferenceOr::Item(openapi::Parameter::Header {
                    parameter_data,
                    style: openapi::HeaderStyle::Simple,
                }) => {
                    if let Err(e) = enforce_string_parameter(&parameter_data) {
                        tracing::warn!("unsupported header parameter: {e}");
                        return None;
                    }

                    header_params.push(HeaderParam {
                        name: parameter_data.name,
                        required: parameter_data.required,
                    });
                }
                ReferenceOr::Item(openapi::Parameter::Query {
                    parameter_data,
                    allow_reserved: false,
                    style: openapi::QueryStyle::Form,
                    allow_empty_value: None,
                }) => {
                    query_params.push(parameter_data);
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

        let request_body_schema_name = op.request_body.and_then(|b| match b {
            ReferenceOr::Item(mut req_body) => {
                assert!(req_body.required);
                assert!(req_body.extensions.is_empty());
                assert_eq!(req_body.content.len(), 1);
                let json_body = req_body
                    .content
                    .swap_remove("application/json")
                    .expect("should have JSON body");
                assert!(json_body.extensions.is_empty());
                match json_body.schema.expect("no json body schema?!").json_schema {
                    Schema::Bool(_) => {
                        tracing::warn!("unexpected bool schema");
                        None
                    }
                    Schema::Object(obj) => {
                        if !obj.is_ref() {
                            tracing::warn!(?obj, "unexpected non-$ref json body schema");
                        }
                        obj.reference.and_then(|r| {
                            let schema_name = r.strip_prefix("#/components/schemas/");
                            if schema_name.is_none() {
                                tracing::warn!(
                                    component_ref = r,
                                    "missing #/components/schemas/ prefix on component ref"
                                );
                            };
                            schema_name.map(ToOwned::to_owned)
                        })
                    }
                }
            }
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
            header_params,
            query_params,
            request_body_schema_name,
        };
        Some((res_name.to_owned(), op))
    }
}

fn enforce_string_parameter(parameter_data: &openapi::ParameterData) -> anyhow::Result<()> {
    let openapi::ParameterSchemaOrContent::Schema(s) = &parameter_data.format else {
        bail!("found unexpected 'content' data format");
    };
    let Schema::Object(obj) = &s.json_schema else {
        bail!("found unexpected `true` schema");
    };
    if obj.instance_type != Some(InstanceType::String.into()) {
        bail!("unsupported path parameter type `{:?}`", obj.instance_type);
    }

    Ok(())
}

#[derive(Debug)]
struct HeaderParam {
    name: String,
    required: bool,
}
