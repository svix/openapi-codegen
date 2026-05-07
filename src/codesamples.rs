use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet, btree_map},
    sync::Arc,
};

use crate::{
    CodegenLanguage,
    api::{
        Api, Resource,
        types::{EnumVariantType, Field, FieldType, StructEnumRepr, Type, TypeData},
    },
    cli_v1::IncludeMode,
    template,
};
use aide::openapi::OpenApi;
use anyhow::Context;
use minijinja::{Value, context};
use serde::{Serialize, Serializer};

fn codesample_env(
    path_param_to_example: Arc<fn(String) -> String>,
) -> Result<minijinja::Environment<'static>, minijinja::Error> {
    let mut env = template::populate_env(minijinja::Environment::new())?;
    env.set_debug(true);

    let path_param_fn = path_param_to_example.clone();
    env.add_filter(
        // given a path param name (for example `app_id`) return the same example id each time
        "path_param_example",
        move |path_param: Cow<'_, str>| -> Result<String, minijinja::Error> {
            Ok(path_param_fn(path_param.to_string()))
        },
    );

    let path_param_fn = path_param_to_example.clone();
    env.add_filter(
        // format a path string `/api/v1/app/{{app_id}}` => `/api/v1/app/app_1srOrx2ZWZBpBUvZwXKQmoEYga2`
        "populate_path_with_examples",
        move |s: Cow<'_, str>, path_params: &Vec<Value>| -> Result<String, minijinja::Error> {
            let mut path_str = s.to_string();
            for field in path_params {
                let field = field.as_str().expect("Expected this to be a string");
                path_str =
                    path_str.replace(&format!("{{{field}}}"), &path_param_fn(field.to_string()));
            }
            Ok(path_str)
        },
    );
    Ok(env)
}

fn recursively_resolve_type(ty_name: &str, api: &Api) -> Type {
    let mut ty = api.types.get(ty_name).unwrap().clone();

    let update_fields = |fields: &mut Vec<Field>, api: &Api| {
        for f in fields.iter_mut() {
            if let FieldType::SchemaRef { name, .. } = &f.r#type {
                let inner_ty = recursively_resolve_type(name, api);
                f.r#type = FieldType::SchemaRef {
                    name: name.clone(),
                    inner: Some(inner_ty),
                };
            }
        }
    };
    match ty.data {
        TypeData::Struct { ref mut fields } => {
            update_fields(fields, api);
        }
        TypeData::StringEnum { .. } => (),
        TypeData::IntegerEnum { .. } => (),
        TypeData::StructEnum {
            ref mut fields,
            ref mut repr,
            ..
        } => {
            match repr {
                StructEnumRepr::AdjacentlyTagged { variants, .. } => {
                    for v in variants.iter_mut() {
                        match &mut v.content {
                            EnumVariantType::Struct { fields } => {
                                update_fields(fields, api);
                            }
                            EnumVariantType::Ref { schema_ref, inner } => {
                                if let Some(schema_ref) = schema_ref {
                                    let inner_ty = recursively_resolve_type(schema_ref, api);
                                    *inner = Some(inner_ty);
                                }
                            }
                        }
                    }
                }
            }

            update_fields(fields, api);
        }
    }
    ty
}

fn gen_samples_for_resource(
    env: &minijinja::Environment<'static>,
    samples_map: &mut BTreeMap<String, Vec<CodeSample>>,
    api: &Api,
    resource: &Resource,
    resource_parents: &Vec<String>,
    templates: &CodesampleTemplates,
) {
    for operation in &resource.operations {
        let btree_map::Entry::Vacant(map_entry) = samples_map.entry(operation.id.clone()) else {
            tracing::error!(operation.id, "duplicate operation ID?");
            continue;
        };

        let samples = templates
            .templates
            .iter()
            .cloned()
            .map(|tpl| {
                let req_body_ty = operation
                    .request_body_schema_name
                    .as_ref()
                    .map(|req_body_name| recursively_resolve_type(req_body_name, api));
                let ctx = context! { operation, resource_parents, req_body_ty };

                let codesample = env.render_str(&tpl.source, ctx).unwrap();
                CodeSample {
                    source: codesample,
                    lang: tpl.lang,
                    label: tpl.label,
                }
            })
            .collect();

        map_entry.insert(samples);
    }

    for (subresource_name, subresource) in &resource.subresources {
        let mut new_parents = resource_parents.clone();
        new_parents.push(subresource_name.clone());

        gen_samples_for_resource(env, samples_map, api, subresource, &new_parents, templates);
    }
}

/// `x-codeSamples` entry.
///
/// This format is understood by many OpenAPI documentation renderers.
#[derive(Debug, Serialize)]
pub struct CodeSample {
    #[serde(serialize_with = "serialize_codegen_language")]
    pub lang: CodegenLanguage,
    pub label: String,
    pub source: String,
}

fn serialize_codegen_language<S>(lang: &CodegenLanguage, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    // Redocly's documentation links to the GitHub linguish list for this:
    // https://github.com/github-linguist/linguist/blob/main/lib/linguist/popular.yml
    let lang = match lang {
        CodegenLanguage::Python => "Python",
        CodegenLanguage::Rust => "Rust",
        CodegenLanguage::Go => "Go",
        CodegenLanguage::Kotlin => "Kotlin",
        CodegenLanguage::CSharp => "C#",
        CodegenLanguage::Java => "Java",
        CodegenLanguage::TypeScript => "TypeScript",
        CodegenLanguage::Ruby => "Ruby",
        CodegenLanguage::Php => "PHP",
        CodegenLanguage::Shell => "Shell",
        CodegenLanguage::Unknown => "unknown",
    };
    serializer.serialize_str(lang)
}

#[derive(Clone)]
struct SampleTemplate {
    source: String,
    label: String,
    lang: CodegenLanguage,
}

#[derive(Default)]
pub struct CodesampleTemplates {
    templates: Vec<SampleTemplate>,
}

impl CodesampleTemplates {
    pub fn add_template(
        &mut self,
        lang: CodegenLanguage,
        label: impl Into<String>,
        source: impl Into<String>,
    ) {
        self.templates.push(SampleTemplate {
            lang,
            label: label.into(),
            source: source.into(),
        });
    }
}

/// Generate code samples.
///
/// Returns a map of `{ operation ID => code samples }`.
pub async fn generate_codesamples(
    openapi_spec: &OpenApi,
    templates: CodesampleTemplates,
    excluded_operation_ids: BTreeSet<String>,
    path_param_example: fn(String) -> String,
) -> anyhow::Result<BTreeMap<String, Vec<CodeSample>>> {
    let api_ir = crate::api::Api::new(
        openapi_spec
            .paths
            .clone()
            .context("found no endpoints in input spec")?,
        &mut openapi_spec.components.clone().unwrap_or_default(),
        &[],
        IncludeMode::Public,
        &excluded_operation_ids,
        &BTreeSet::new(),
    )?;

    let mut samples_map = BTreeMap::new();

    let env = codesample_env(Arc::new(path_param_example))?;

    for (resource_name, resource) in &api_ir.resources {
        gen_samples_for_resource(
            &env,
            &mut samples_map,
            &api_ir,
            resource,
            &vec![resource_name.clone()],
            &templates,
        );
    }
    Ok(samples_map)
}
