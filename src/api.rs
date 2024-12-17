use std::{
    collections::{BTreeMap, BTreeSet},
    io::BufWriter,
    path::Path,
};

use aide::openapi::{self, ReferenceOr};
use anyhow::{bail, Context as _};
use fs_err::File;
use heck::ToSnakeCase as _;
use indexmap::IndexMap;
use minijinja::context;
use schemars::schema::{InstanceType, Schema};

use crate::{
    template,
    types::{FieldType, Types},
    util::get_schema_name,
};

/// The API we generate a client for.
///
/// Intermediate representation of `paths` from the spec.
#[derive(Debug)]
pub(crate) struct Api {
    resources: BTreeMap<String, Resource>,
}

impl Api {
    pub(crate) fn new(paths: openapi::Paths, with_deprecated: bool) -> anyhow::Result<Self> {
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
                if !with_deprecated && op.deprecated {
                    continue;
                }

                if let Some((res_name, op)) = Operation::from_openapi(&path, method, op) {
                    let resource = resources
                        .entry(res_name.clone())
                        .or_insert_with(|| Resource::new(res_name));
                    resource.operations.push(op);
                }
            }
        }

        Ok(Self { resources })
    }

    fn referenced_components(&self) -> impl Iterator<Item = &str> {
        self.resources
            .values()
            .flat_map(Resource::referenced_components)
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

    pub(crate) fn generate(
        self,
        template_name: &str,
        output_dir: impl AsRef<Path>,
        no_format: bool,
    ) -> anyhow::Result<()> {
        let output_dir = output_dir.as_ref();

        // Use the second `.`-separated segment of the filename, so for
        // `foo.rs.jinja` this get us `rs`, not `jinja`.
        let tpl_file_ext = template_name
            .split('.')
            .nth(1)
            .context("template must have a file extension")?;

        let minijinja_env = template::env()?;
        let tpl = minijinja_env.get_template(template_name)?;

        for (name, resource) in self.resources {
            let filename = format!("{}.{tpl_file_ext}", name.to_snake_case());
            let referenced_components = resource.referenced_components().collect::<BTreeSet<_>>();
            let ctx = context! { resource, referenced_components };

            let file_path = output_dir.join(filename);
            let out_file = BufWriter::new(File::create(&file_path)?);
            tpl.render_to_write(ctx, out_file)?;

            if !no_format {
                run_formatter(&file_path, tpl_file_ext);
            }
        }

        Ok(())
    }
}

fn run_formatter(path: &Path, file_ext: &str) {
    if file_ext == "rs" {
        _ = std::process::Command::new("rustfmt")
            .args(["+nightly", "--edition", "2021"])
            .arg(path)
            .status();
    }
}

/// A named group of [`Operation`]s.
#[derive(Debug, serde::Serialize)]
struct Resource {
    name: String,
    operations: Vec<Operation>,
    // TODO: subresources?
}

impl Resource {
    fn new(name: String) -> Self {
        Self {
            name,
            operations: Vec::new(),
        }
    }

    fn referenced_components(&self) -> impl Iterator<Item = &str> {
        self.operations.iter().flat_map(|operation| {
            operation
                .query_params
                .iter()
                .filter_map(|p| match &p.r#type {
                    FieldType::SchemaRef(r) => Some(r.as_str()),
                    _ => None,
                })
                .chain(operation.request_body_schema_name.as_deref())
                .chain(operation.response_body_schema_name.as_deref())
        })
    }
}

/// A named HTTP endpoint.
#[derive(Debug, serde::Serialize)]
struct Operation {
    /// The operation ID from the spec.
    id: String,
    /// The name to use for the operation in code.
    name: String,
    /// Description of the operation to use for documentation.
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    /// Whether this operation is marked as deprecated.
    deprecated: bool,
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
    query_params: Vec<QueryParam>,
    /// Name of the request body type, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    request_body_schema_name: Option<String>,
    /// Name of the response body type, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    response_body_schema_name: Option<String>,
}

impl Operation {
    #[tracing::instrument(name = "operation_from_openapi", skip(op), fields(op_id))]
    fn from_openapi(path: &str, method: &str, op: openapi::Operation) -> Option<(String, Self)> {
        let Some(op_id) = op.operation_id else {
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
                    let name = parameter_data.name;
                    let _guard = tracing::info_span!("field_type_from_openapi", name).entered();
                    let r#type = match FieldType::from_openapi(parameter_data.format) {
                        Ok(t) => t,
                        Err(e) => {
                            tracing::warn!("unsupport query parameter type: {e}");
                            return None;
                        }
                    };

                    query_params.push(QueryParam {
                        name,
                        description: parameter_data.description,
                        required: parameter_data.required,
                        r#type,
                    });
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
                        tracing::error!("unexpected bool schema");
                        None
                    }
                    Schema::Object(obj) => {
                        if !obj.is_ref() {
                            tracing::error!(?obj, "unexpected non-$ref json body schema");
                        }
                        get_schema_name(obj.reference)
                    }
                }
            }
            ReferenceOr::Reference { .. } => {
                tracing::error!("$ref request bodies are not currently supported");
                None
            }
        });

        let response_body_schema_name = op.responses.and_then(|r| {
            assert_eq!(r.default, None);
            assert!(r.extensions.is_empty());
            let mut success_responses = r.responses.into_iter().filter(|(st, _)| {
                match st {
                    openapi::StatusCode::Code(c) => match c {
                        0..100 => tracing::error!("invalid status code < 100"),
                        100..200 => tracing::error!("what is this? status code {c}..."),
                        200..300 => return true,
                        300..400 => tracing::error!("what is this? status code {c}..."),
                        400.. => {}
                    },
                    openapi::StatusCode::Range(_) => {
                        tracing::error!("unsupported status code range");
                    }
                }

                false
            });

            let (_, resp) = success_responses
                .next()
                .expect("every operation must have one success response");
            let schema_name = response_body_schema_name(resp);
            for (_, resp) in success_responses {
                assert_eq!(schema_name, response_body_schema_name(resp));
            }

            schema_name
        });

        let res_name = res_name.to_owned();
        let op_name = op_name.to_owned();

        let op = Operation {
            id: op_id,
            name: op_name,
            description: op.description,
            deprecated: op.deprecated,
            method: method.to_owned(),
            path: path.to_owned(),
            path_params,
            header_params,
            query_params,
            request_body_schema_name,
            response_body_schema_name,
        };
        Some((res_name, op))
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

fn response_body_schema_name(resp: ReferenceOr<openapi::Response>) -> Option<String> {
    match resp {
        ReferenceOr::Item(mut resp_body) => {
            assert!(resp_body.extensions.is_empty());
            if resp_body.content.is_empty() {
                return None;
            }

            assert_eq!(resp_body.content.len(), 1);
            let json_body = resp_body
                .content
                .swap_remove("application/json")
                .expect("should have JSON body");
            assert!(json_body.extensions.is_empty());
            match json_body.schema.expect("no json body schema?!").json_schema {
                Schema::Bool(_) => {
                    tracing::error!("unexpected bool schema");
                    None
                }
                Schema::Object(obj) => {
                    if !obj.is_ref() {
                        tracing::error!(?obj, "unexpected non-$ref json body schema");
                    }
                    get_schema_name(obj.reference)
                }
            }
        }
        ReferenceOr::Reference { .. } => {
            tracing::error!("$ref response bodies are not currently supported");
            None
        }
    }
}

#[derive(Debug, serde::Serialize)]
struct HeaderParam {
    name: String,
    required: bool,
}

#[derive(Debug, serde::Serialize)]
struct QueryParam {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    required: bool,
    r#type: FieldType,
}
