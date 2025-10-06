use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use crate::{
    CodegenLanguage,
    api::{
        Api, Resource,
        types::{EnumVariantType, Field, FieldType, StructEnumRepr, Type, TypeData},
    },
    template,
};
use aide::openapi::OpenApi;
use anyhow::Context;
use minijinja::{Value, context};
use serde::Serialize;

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
                let inner_ty = api.types.get(name).unwrap().clone();
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
                                    let inner_ty = api.types.get(schema_ref).unwrap().clone();
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

fn generate_sample(
    env: &minijinja::Environment<'static>,
    samples_map: &mut BTreeMap<CodegenLanguage, Vec<CodeSample>>,
    api: &Api,
    resource: &Resource,
    resource_parents: &Vec<String>,
    templates: &CodesampleTemplates,
) {
    for operation in &resource.operations {
        for SampleTemplate {
            source,
            label,
            formatting_lang,
            lang_name,
        } in &templates.templates
        {
            let req_body_ty = operation
                .request_body_schema_name
                .as_ref()
                .map(|req_body_name| recursively_resolve_type(req_body_name, api));

            let ctx = context! { operation, resource_parents, req_body_ty };

            let codesample = env.render_str(source, ctx).unwrap();
            let sample = CodeSample {
                lang: lang_name.to_string(),
                source: codesample,
                formatting_lang: *formatting_lang,
                op_id: operation.id.clone(),
                label: label.clone(),
            };

            let lang_vec = match samples_map.get_mut(formatting_lang) {
                Some(v) => v,
                None => {
                    samples_map.insert(*formatting_lang, vec![]);
                    samples_map.get_mut(formatting_lang).unwrap()
                }
            };

            lang_vec.push(sample);
        }
    }

    for (subresource_name, subresource) in &resource.subresources {
        let mut new_parents = resource_parents.clone();
        new_parents.push(subresource_name.clone());

        generate_sample(env, samples_map, api, subresource, &new_parents, templates);
    }
}

#[derive(Debug, Serialize, Clone)]
pub struct CodeSample {
    pub source: String,
    pub lang: String,
    pub label: String,
    #[serde(skip)]
    pub op_id: String,
    #[serde(skip)]
    pub formatting_lang: CodegenLanguage,
}

struct SampleTemplate {
    source: String,
    label: String,
    lang_name: String,
    formatting_lang: CodegenLanguage,
}

#[derive(Default)]
pub struct CodesampleTemplates {
    templates: Vec<SampleTemplate>,
}

impl CodesampleTemplates {
    pub fn add_template<S: AsRef<str>>(
        &mut self,
        label: S,
        lang_name: S,
        formatting_lang: CodegenLanguage,
        source: S,
    ) {
        self.templates.push(SampleTemplate {
            formatting_lang,
            lang_name: lang_name.as_ref().to_string(),
            label: label.as_ref().to_string(),
            source: source.as_ref().to_string(),
        });
    }
}

pub async fn generate_codesamples(
    openapi_spec: &str,
    templates: CodesampleTemplates,
    excluded_operation_ids: BTreeSet<String>,
    path_param_example: fn(String) -> String,
) -> anyhow::Result<BTreeMap<CodegenLanguage, Vec<CodeSample>>> {
    let openapi_spec: OpenApi =
        serde_json::from_str(openapi_spec).context("failed to parse OpenAPI spec")?;

    let api_ir = crate::api::Api::new(
        openapi_spec
            .paths
            .expect("found no endpoints in input spec"),
        &mut openapi_spec.components.unwrap_or_default(),
        &[],
        crate::IncludeMode::OnlyPublic,
        &excluded_operation_ids,
        &BTreeSet::new(),
    )?;

    let mut samples_map = BTreeMap::new();

    let env = codesample_env(Arc::new(path_param_example))?;

    for (resource_name, resource) in &api_ir.resources {
        generate_sample(
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
