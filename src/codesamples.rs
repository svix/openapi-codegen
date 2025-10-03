use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::{
    CodegenLanguage,
    api::{Api, Field, FieldType, Resource, Type},
    docker_postprocessing::ContainerizedPostprocessor,
    template,
};
use aide::openapi::OpenApi;
use camino::Utf8PathBuf;
use minijinja::context;
use serde::Serialize;
use tempfile::tempdir;
use tokio::task::JoinSet;

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
        crate::api::TypeData::Struct { ref mut fields } => {
            update_fields(fields, api);
        }
        crate::api::TypeData::StringEnum { .. } => (),
        crate::api::TypeData::IntegerEnum { .. } => (),
        crate::api::TypeData::StructEnum {
            ref mut fields,
            ref mut repr,
            ..
        } => {
            match repr {
                crate::api::StructEnumRepr::AdjacentlyTagged { variants, .. } => {
                    for v in variants.iter_mut() {
                        match &mut v.content {
                            crate::api::EnumVariantType::Struct { fields } => {
                                update_fields(fields, api);
                            }
                            crate::api::EnumVariantType::Ref { schema_ref, inner } => {
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
    samples_map: &mut BTreeMap<CodegenLanguage, Vec<CodeSample>>,
    api: &Api,

    resource: &Resource,
    resource_parents: &Vec<String>,
    templates: &CodesampleTemplates,
) {
    let env = template::populate_env(minijinja::Environment::new()).unwrap();

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
                lang_name: lang_name.to_string(),
                code: codesample,
                formatting_lang: *formatting_lang,
                op_id: operation.id.clone(),
                sample_id: uuid::Uuid::new_v4(),
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

        generate_sample(samples_map, api, subresource, &new_parents, templates);
    }
}

#[derive(Debug, Serialize, Clone)]
pub struct CodeSample {
    #[serde(rename = "source")]
    pub code: String,
    #[serde(skip)]
    pub formatting_lang: CodegenLanguage,
    // name for language
    #[serde(rename = "lang")]
    pub lang_name: String,
    pub label: String,
    #[serde(skip)]
    pub op_id: String,
    #[serde(skip)]
    pub sample_id: uuid::Uuid,
}

impl CodeSample {
    fn filename(&self) -> String {
        format!("{}.{}", self.sample_id, self.formatting_lang.ext())
    }
}

pub trait CodeSampleTemplate {
    fn template() -> String;
    fn lang() -> CodegenLanguage;
}

struct SampleTemplate {
    source: String,
    label: String,
    lang_name: String,
    formatting_lang: CodegenLanguage,
}

pub struct CodesampleTemplates {
    templates: Vec<SampleTemplate>,
}

impl Default for CodesampleTemplates {
    fn default() -> Self {
        Self::new()
    }
}

impl CodesampleTemplates {
    pub fn new() -> Self {
        Self { templates: vec![] }
    }
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
    openapi_spec: OpenApi,
    templates: CodesampleTemplates,
) -> anyhow::Result<BTreeMap<String, Vec<CodeSample>>> {
    let mut e = BTreeSet::new();
    e.insert("v1.health.get".to_string());
    let api_ir = crate::api::Api::new(
        openapi_spec
            .paths
            .expect("found no endpoints in input spec"),
        &mut openapi_spec.components.unwrap_or_default(),
        &[],
        crate::IncludeMode::OnlyPublic,
        &e,
        &BTreeSet::new(),
    )
    .unwrap();

    let mut samples_map = BTreeMap::new();

    for (resource_name, resource) in &api_ir.resources {
        generate_sample(
            &mut samples_map,
            &api_ir,
            resource,
            &vec![resource_name.clone()],
            &templates,
        );
    }
    let samples_map = format_samples(samples_map).await?;
    Ok(samples_map)
}

// TODO: find a better way to do this, but for now this is what I will do
async fn format_samples(
    samples_map: BTreeMap<CodegenLanguage, Vec<CodeSample>>,
) -> anyhow::Result<BTreeMap<String, Vec<CodeSample>>> {
    let mut join_set = JoinSet::new();
    let mut op_id_to_samples = BTreeMap::new();

    for (lang, samples) in samples_map {
        if lang == CodegenLanguage::Rust {
            for mut sample in samples {
                let sample_vec = match op_id_to_samples.get_mut(&sample.op_id) {
                    Some(v) => v,
                    None => {
                        op_id_to_samples.insert(sample.op_id.clone(), vec![]);
                        op_id_to_samples.get_mut(&sample.op_id).unwrap()
                    }
                };

                let file =
                    syn::parse_file(&add_fmt_boilerplate(&sample.code, CodegenLanguage::Rust))
                        .unwrap();

                let formatted = prettyplease::unparse(&file);
                sample.code = strip_fmt_boilerplate(&formatted, CodegenLanguage::Rust);
                sample_vec.push(sample);
            }
            continue;
        }
        let task = async move || -> anyhow::Result<BTreeMap<String, Vec<CodeSample>>> {
            let mut op_id_to_samples = BTreeMap::new();
            let tmpdir = tempdir()?;
            let tmpdir_path = tmpdir.path().to_path_buf();

            let mut paths = vec![];
            for sample in &samples {
                let sample_path =
                    Utf8PathBuf::from_path_buf(tmpdir_path.join(sample.filename())).unwrap();
                paths.push(sample_path.clone());

                std::fs::write(sample_path, add_fmt_boilerplate(&sample.code, lang)).unwrap();
            }

            let p = ContainerizedPostprocessor::new(
                lang,
                tmpdir_path.to_path_buf().try_into().unwrap(),
                &paths,
            );

            match p.run_postprocessor().await {
                Ok(_) => (),
                Err(e) => {
                    let k = tmpdir.keep();
                    dbg!(k);
                    panic!("{e:?}");
                }
            };

            for mut sample in samples {
                let sample_vec = match op_id_to_samples.get_mut(&sample.op_id) {
                    Some(v) => v,
                    None => {
                        op_id_to_samples.insert(sample.op_id.clone(), vec![]);
                        op_id_to_samples.get_mut(&sample.op_id).unwrap()
                    }
                };

                let sample_path =
                    Utf8PathBuf::from_path_buf(tmpdir_path.join(sample.filename())).unwrap();

                let formatted_code =
                    strip_fmt_boilerplate(&std::fs::read_to_string(sample_path).unwrap(), lang);
                sample.code = formatted_code;

                sample_vec.push(sample);
            }
            Ok(op_id_to_samples)
        };
        join_set.spawn(task());
    }

    for map in join_set.join_all().await {
        for (id, mut samples) in map? {
            let vec_to_append = match op_id_to_samples.get_mut(&id) {
                Some(v) => v,
                None => {
                    op_id_to_samples.insert(id.clone(), vec![]);
                    op_id_to_samples.get_mut(&id).unwrap()
                }
            };
            vec_to_append.append(&mut samples);
        }
    }

    for v in op_id_to_samples.values_mut() {
        sort_samples(v);
    }
    Ok(op_id_to_samples)
}

fn add_fmt_boilerplate(sample_text: &str, lang: CodegenLanguage) -> String {
    match lang {
        CodegenLanguage::Go => {
            format!("package main\n\nfunc main() {{ {sample_text} }}\n")
        }
        CodegenLanguage::Rust => {
            format!(r"async fn main() {{ {sample_text} }}")
        }
        _ => sample_text.to_string(),
    }
}

fn strip_fmt_boilerplate(sample_text: &str, lang: CodegenLanguage) -> String {
    match lang {
        CodegenLanguage::Go => unindent(
            sample_text
                .strip_prefix("package main\n\nfunc main() {")
                .unwrap()
                .strip_suffix("}\n")
                .unwrap(),
        ),
        CodegenLanguage::Rust => unindent(
            sample_text
                .strip_prefix("async fn main() {\n")
                .unwrap()
                .strip_suffix("}\n")
                .unwrap(),
        ),
        _ => sample_text.to_string(),
    }
}

fn unindent(s: &str) -> String {
    let lines: Vec<&str> = s.lines().collect();

    // Find the minimum indentation (excluding empty lines)
    let min_indent = lines
        .iter()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.len() - line.trim_start().len())
        .min()
        .unwrap_or(0);

    // Remove the minimum indentation from each line
    lines
        .iter()
        .map(|line| {
            if line.trim().is_empty() {
                // Preserve empty lines as empty
                ""
            } else {
                // Remove min_indent characters from the start
                &line[min_indent.min(line.len())..]
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn sort_samples(samples: &mut [CodeSample]) {
    let sort_order = vec![
        "JavaScript",
        "TypeScript",
        "Python",
        "Python (Async)",
        "Go",
        "Kotlin",
        "Java",
        "Ruby",
        "Rust",
        "C#",
        "PHP",
        "CLI",
        "cURL",
    ];
    let order_map: HashMap<&str, usize> = sort_order
        .iter()
        .enumerate()
        .map(|(i, &s)| (s, i))
        .collect();

    // Sort with custom logic
    samples.sort_by(|a, b| {
        let a_pos = order_map.get(a.label.as_str());
        let b_pos = order_map.get(b.label.as_str());

        match (a_pos, b_pos) {
            // Both in custom order: compare positions
            (Some(&pos_a), Some(&pos_b)) => pos_a.cmp(&pos_b),
            // Only 'a' in custom order: 'a' comes first
            (Some(_), None) => std::cmp::Ordering::Less,
            // Only 'b' in custom order: 'b' comes first
            (None, Some(_)) => std::cmp::Ordering::Greater,
            // Neither in custom order: sort alphabetically
            (None, None) => a.label.cmp(&b.label),
        }
    });
}
