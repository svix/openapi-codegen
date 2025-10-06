use std::str::FromStr;

use anyhow::{Context as _, bail};
use camino::{Utf8Path, Utf8PathBuf};
use fs_err as fs;
use heck::{ToLowerCamelCase, ToSnakeCase as _, ToUpperCamelCase as _};
use minijinja::{Template, context};
use serde::Deserialize;

use crate::{
    api::{Api, Resource},
    postprocessing::Postprocessor,
    template,
};

#[derive(Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum TemplateKind {
    #[default]
    ApiResource,
    OperationOptions,
    Type,
    Summary,
}

pub(crate) fn generate(
    api: Api,
    tpl_name: String,
    output_dir: &Utf8Path,
    no_postprocess: bool,
) -> anyhow::Result<Vec<Utf8PathBuf>> {
    let (name_without_jinja_suffix, tpl_path) = match tpl_name.strip_suffix(".jinja") {
        Some(basename) => (basename, &tpl_name),
        None => (tpl_name.as_str(), &format!("{tpl_name}.jinja")),
    };

    let (tpl_base_name, tpl_file_ext) = Utf8Path::new(name_without_jinja_suffix)
        .file_name()
        .context("template name must not end in '/'")?
        .rsplit_once(".")
        .context("template name must contain '.'")?;

    let tpl_kind = match tpl_base_name {
        "api_resource" => TemplateKind::ApiResource,
        "operation_options" => TemplateKind::OperationOptions,
        "api_summary" | "component_type_summary" | "summary" => TemplateKind::Summary,
        "component_type" => TemplateKind::Type,
        _ => bail!(
            "template file basename must be one of 'api_resource', 'api_summary', \
             'component_type', 'component_type_summary', 'summary'",
        ),
    };

    let tpl_source = fs::read_to_string(tpl_path)?;

    let mut minijinja_env = template::env_with_dir(
        Utf8Path::new(tpl_path)
            .parent()
            .with_context(|| format!("invalid template path `{tpl_path}`"))?,
    )?;
    minijinja_env.add_template(tpl_path, &tpl_source)?;
    let tpl = minijinja_env.get_template(tpl_path)?;

    fs::create_dir_all(output_dir)?;

    let generator = Generator {
        tpl,
        tpl_file_ext,
        output_dir,
    };

    let generated_paths = match tpl_kind {
        TemplateKind::OperationOptions => generator.generate_api_resources_options(api)?,
        TemplateKind::ApiResource => generator.generate_api_resources(api)?,
        TemplateKind::Type => generator.generate_types(api, output_dir)?,
        TemplateKind::Summary => generator.generate_summary(api)?,
    };

    if !no_postprocess {
        let postprocessor = Postprocessor::from_ext(tpl_file_ext, output_dir, &generated_paths);
        postprocessor.run_postprocessor()?;
    }

    Ok(generated_paths)
}

struct Generator<'a> {
    tpl: Template<'a, 'a>,
    tpl_file_ext: &'a str,
    output_dir: &'a Utf8Path,
}

impl Generator<'_> {
    fn generate_api_resources_options(self, api: Api) -> anyhow::Result<Vec<Utf8PathBuf>> {
        self.generate_api_resources_options_inner(api.resources.values())
    }

    fn generate_api_resources_options_inner<'a>(
        &self,
        resources: impl Iterator<Item = &'a Resource>,
    ) -> anyhow::Result<Vec<Utf8PathBuf>> {
        let mut generated_paths = vec![];
        for resource in resources {
            let referenced_components = resource.referenced_components();
            for operation in &resource.operations {
                if operation.has_query_or_header_params() {
                    generated_paths.extend_from_slice(&self.render_tpl(
                        Some(&format!("{}_{}_Options", resource.name, operation.name)),
                        context! { operation, resource, referenced_components },
                    )?);
                }
            }

            generated_paths.extend_from_slice(
                &self.generate_api_resources_options_inner(resource.subresources.values())?,
            );
        }

        Ok(generated_paths)
    }

    fn generate_api_resources(self, api: Api) -> anyhow::Result<Vec<Utf8PathBuf>> {
        self.generate_api_resources_inner(api.resources.values())
    }

    fn generate_api_resources_inner<'a>(
        &self,
        resources: impl Iterator<Item = &'a Resource>,
    ) -> anyhow::Result<Vec<Utf8PathBuf>> {
        let mut generated_paths = vec![];

        for resource in resources {
            let referenced_components = resource.referenced_components();
            generated_paths.extend_from_slice(&self.render_tpl(
                Some(&resource.name),
                context! { resource, referenced_components },
            )?);
            generated_paths.extend_from_slice(
                &self.generate_api_resources_inner(resource.subresources.values())?,
            );
        }

        Ok(generated_paths)
    }

    fn generate_types(self, api: Api, output_dir: &Utf8Path) -> anyhow::Result<Vec<Utf8PathBuf>> {
        let mut generated_paths = vec![];

        let output_dir = output_dir.as_str();
        for (name, ty) in api.types {
            let referenced_components = ty.referenced_components();
            generated_paths.extend_from_slice(&self.render_tpl(
                Some(&name),
                context! { type => ty, referenced_components, output_dir },
            )?);
        }

        Ok(generated_paths)
    }

    fn generate_summary(&self, api: Api) -> anyhow::Result<Vec<Utf8PathBuf>> {
        self.render_tpl(None, context! { api })
    }

    fn render_tpl(
        &self,
        output_name: Option<&str>,
        ctx: minijinja::Value,
    ) -> anyhow::Result<Vec<Utf8PathBuf>> {
        let mut generated_paths = vec![];

        let tpl_file_ext = self.tpl_file_ext;
        let basename = match (output_name, tpl_file_ext) {
            (Some(name), "ts") => name.to_lower_camel_case(),
            (Some(name), "cs" | "java" | "kt" | "php") => name.to_upper_camel_case(),
            (Some(name), _) => name.to_snake_case(),
            (None, "py") => "__init__".to_owned(),
            (None, "rs") => "mod".to_owned(),
            (None, "cs" | "java" | "kt") => "Summary".to_owned(),
            (None, "ts") => "index".to_owned(),
            (None, "go") => "models".to_owned(),
            (None, "rb") => "svix".to_owned(),
            (None, "php") => "Svix".to_owned(),
            (None, _) => "summary".to_owned(),
        };

        let (rendered_data, state) = self.tpl.render_and_return_state(ctx)?;

        let file_path = match state.get_temp("summary_filename") {
            Some(summary_filename) => self.output_dir.join(summary_filename.as_str().unwrap()),
            None => self.output_dir.join(format!("{basename}.{tpl_file_ext}")),
        };

        generated_paths.push(file_path.clone());
        fs::write(&file_path, rendered_data)?;

        if let Some(extra_generated_file) = state.get_temp("extra_generated_file") {
            let extra_generated_filepath =
                Utf8PathBuf::from_str(extra_generated_file.as_str().unwrap())?;
            generated_paths.push(extra_generated_filepath);
        }

        Ok(generated_paths)
    }
}
