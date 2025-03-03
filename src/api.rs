use std::collections::{BTreeMap, BTreeSet};

use aide::openapi::{self, ReferenceOr};
use anyhow::{bail, Context as _};
use indexmap::IndexMap;
use schemars::schema::{InstanceType, Schema};
use serde::Serialize;

use crate::{
    types::{FieldType, Types},
    util::{get_schema_name, serialize_btree_map_values},
};

/// The API we generate a client for.
///
/// Intermediate representation of `paths` from the spec.
#[derive(Debug, Serialize)]
pub(crate) struct Api {
    #[serde(serialize_with = "serialize_btree_map_values")]
    pub resources: BTreeMap<String, Resource>,
}

impl Api {
    pub(crate) fn new(
        paths: openapi::Paths,
        component_schemas: &IndexMap<String, openapi::SchemaObject>,
        include_hidden: bool,
    ) -> anyhow::Result<Self> {
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
                if let Some((res_path, op)) =
                    Operation::from_openapi(&path, method, op, component_schemas, include_hidden)
                {
                    let resource = get_or_insert_resource(&mut resources, res_path);
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
        Types::from_referenced_components(schemas, self.referenced_components())
    }
}

fn get_or_insert_resource(
    resources: &mut BTreeMap<String, Resource>,
    path: Vec<String>,
) -> &mut Resource {
    let mut path_iter = path.into_iter();
    let mut name = path_iter.next().expect("path must be non-empty");
    let mut r = resources
        .entry(name.clone())
        .or_insert_with(|| Resource::new(name.clone()));

    for sub_name in path_iter {
        name.push('-');
        name.push_str(&sub_name);

        r = r
            .subresources
            .entry(sub_name)
            .or_insert_with(|| Resource::new(name.clone()));
    }

    r
}

/// A named group of [`Operation`]s.
#[derive(Debug, serde::Serialize)]
pub(crate) struct Resource {
    pub name: String,
    pub operations: Vec<Operation>,
    pub subresources: BTreeMap<String, Resource>,
}

impl Resource {
    fn new(name: String) -> Self {
        Self {
            name,
            operations: Vec::new(),
            subresources: BTreeMap::new(),
        }
    }

    pub(crate) fn referenced_components(&self) -> BTreeSet<&str> {
        let mut res = BTreeSet::new();

        for resource in self.subresources.values() {
            res.extend(resource.referenced_components());
        }

        for operation in &self.operations {
            for param in &operation.query_params {
                if let FieldType::SchemaRef(name) = &param.r#type {
                    res.insert(name);
                }
            }
            if let Some(name) = &operation.request_body_schema_name {
                res.insert(name);
            }
            if let Some(name) = &operation.response_body_schema_name {
                res.insert(name);
            }
        }

        res
    }
}

/// A named HTTP endpoint.
#[derive(Debug, serde::Serialize)]
pub(crate) struct Operation {
    /// The operation ID from the spec.
    id: String,
    /// The name to use for the operation in code.
    pub(crate) name: String,
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
    /// Some request bodies are required, but all the fields are optional (i.e. the CLI can omit
    /// this from the argument list).
    /// Only useful when `request_body_schema_name` is `Some`.
    request_body_all_optional: bool,
    /// Name of the response body type, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    response_body_schema_name: Option<String>,
}

impl Operation {
    #[tracing::instrument(
        name = "operation_from_openapi",
        skip_all,
        fields(path = path, method = method, op_id),
    )]
    fn from_openapi(
        path: &str,
        method: &str,
        op: openapi::Operation,
        component_schemas: &IndexMap<String, aide::openapi::SchemaObject>,
        include_hidden: bool,
    ) -> Option<(Vec<String>, Self)> {
        let Some(op_id) = op.operation_id else {
            // ignore operations without an operationId
            return None;
        };
        tracing::Span::current().record("op_id", &op_id);

        if !include_hidden && op.extensions.get("x-hidden").is_some_and(|val| val == true) {
            return None;
        }

        let mut op_id_parts_iter = op_id.split('.');
        let version = op_id_parts_iter
            .next()
            .expect("split iter always contains at least one item");
        let Some(op_name) = op_id_parts_iter.next_back() else {
            tracing::debug!("skipping operation whose ID doesn't contain a period");
            return None;
        };

        let res_path: Vec<_> = op_id_parts_iter.map(ToOwned::to_owned).collect();
        if res_path.is_empty() {
            tracing::debug!("skipping operation whose ID only contains one period");
            return None;
        }

        if version != "v1" {
            tracing::warn!("found operation whose ID does not begin with v1");
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
                    if parameter_data.name != "idempotency-key" {
                        tracing::warn!(name = parameter_data.name, "unknown header parameter");
                    }

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
                    if method == "post" && name == "get_if_exists" {
                        tracing::debug!("ignoring get_if_exists query parameter");
                        continue;
                    }

                    let _guard = tracing::info_span!("field_type_from_openapi", name).entered();
                    let r#type = match FieldType::from_openapi(parameter_data.format) {
                        Ok(t) => t,
                        Err(e) => {
                            tracing::warn!("unsupported query parameter type: {e}");
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

        let request_body_all_optional = op
            .request_body
            .as_ref()
            .map(|r| {
                match r {
                    ReferenceOr::Reference { .. } => {
                        unimplemented!("reference")
                    }
                    ReferenceOr::Item(body) => {
                        if let Some(mt) = body.content.get("application/json") {
                            match mt.schema.as_ref().map(|so| &so.json_schema) {
                                Some(Schema::Object(schemars::schema::SchemaObject {
                                    object: Some(ov),
                                    ..
                                })) => {
                                    return ov.required.is_empty();
                                }
                                Some(Schema::Object(schemars::schema::SchemaObject {
                                    reference: Some(s),
                                    ..
                                })) => {
                                    match component_schemas
                                        .get(
                                            &get_schema_name(Some(s)).expect("schema should exist"),
                                        )
                                        .map(|so| &so.json_schema)
                                    {
                                        Some(Schema::Object(schemars::schema::SchemaObject {
                                            object: Some(ov),
                                            ..
                                        })) => {
                                            return ov.required.is_empty();
                                        }
                                        _ => unimplemented!("double ref not supported"),
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
                false
            })
            .unwrap_or_default();

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
                        get_schema_name(obj.reference.as_deref())
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
            request_body_all_optional,
            response_body_schema_name,
        };
        Some((res_path, op))
    }

    pub(crate) fn has_query_or_header_params(&self) -> bool {
        !self.header_params.is_empty() || !self.query_params.is_empty()
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
                    get_schema_name(obj.reference.as_deref())
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
