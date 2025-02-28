use std::io::BufWriter;

use anyhow::{bail, Context as _};
use camino::Utf8Path;
use fs_err::{self as fs, File};
use heck::{ToLowerCamelCase, ToSnakeCase as _, ToUpperCamelCase as _};
use minijinja::{context, Template};
use serde::Deserialize;

use crate::{
    api::{Api, Resource},
    postprocessing::Postprocessor,
    template,
    types::Types,
    PostprocessorOptions,
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

pub(crate) async fn generate(
    api: Api,
    types: Types,
    tpl_name: String,
    output_dir: &Utf8Path,
    postprocessor_options: PostprocessorOptions,
) -> anyhow::Result<()> {
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

    let mut minijinja_env = template::env(
        Utf8Path::new(tpl_path)
            .parent()
            .with_context(|| format!("invalid template path `{tpl_path}`"))?,
    )?;
    minijinja_env.add_template(tpl_path, &tpl_source)?;
    let tpl = minijinja_env.get_template(tpl_path)?;

    let postprocessor = Postprocessor::from_ext(tpl_file_ext, output_dir, &postprocessor_options);

    let generator = Generator {
        tpl,
        tpl_file_ext,
        output_dir,
        postprocessor: &postprocessor,
        no_postprocess: postprocessor_options.no_postprocess,
    };

    match tpl_kind {
        TemplateKind::OperationOptions => generator.generate_api_resources_options(api)?,
        TemplateKind::ApiResource => generator.generate_api_resources(api)?,
        TemplateKind::Type => generator.generate_types(types)?,
        TemplateKind::Summary => generator.generate_summary(types, api)?,
    }

    if !postprocessor_options.no_postprocess {
        postprocessor.run_postprocessor().await?;
    }

    Ok(())
}

struct Generator<'a> {
    tpl: Template<'a, 'a>,
    tpl_file_ext: &'a str,
    output_dir: &'a Utf8Path,
    postprocessor: &'a Postprocessor,
    no_postprocess: bool,
}

impl Generator<'_> {
    fn generate_api_resources_options(self, api: Api) -> anyhow::Result<()> {
        self.generate_api_resources_options_inner(api.resources.values())
    }

    fn generate_api_resources_options_inner<'a>(
        &self,
        resources: impl Iterator<Item = &'a Resource>,
    ) -> anyhow::Result<()> {
        for resource in resources {
            let referenced_components = resource.referenced_components();
            for operation in &resource.operations {
                if operation.has_query_or_header_params() {
                    self.render_tpl(
                        Some(&format!("{}_{}_Options", resource.name, operation.name)),
                        context! { operation, resource, referenced_components },
                    )?;
                }
            }

            self.generate_api_resources_options_inner(resource.subresources.values())?;
        }

        Ok(())
    }

    fn generate_api_resources(self, api: Api) -> anyhow::Result<()> {
        self.generate_api_resources_inner(api.resources.values())
    }

    fn generate_api_resources_inner<'a>(
        &self,
        resources: impl Iterator<Item = &'a Resource>,
    ) -> anyhow::Result<()> {
        for resource in resources {
            let referenced_components = resource.referenced_components();
            if resource.has_operations() {
                self.render_tpl(
                    Some(&resource.name),
                    context! { resource, referenced_components },
                )?;
            }
            self.generate_api_resources_inner(resource.subresources.values())?;
        }

        Ok(())
    }

    fn generate_types(self, Types(types): Types) -> anyhow::Result<()> {
        for (name, ty) in types {
            let referenced_components = ty.referenced_components();
            self.render_tpl(Some(&name), context! { type => ty, referenced_components })?;
        }

        Ok(())
    }

    fn generate_summary(&self, Types(types): Types, api: Api) -> anyhow::Result<()> {
        self.render_tpl(None, context! { types, api })
    }

    fn render_tpl(&self, output_name: Option<&str>, ctx: minijinja::Value) -> anyhow::Result<()> {
        let tpl_file_ext = self.tpl_file_ext;
        let basename = match (output_name, tpl_file_ext) {
            (Some(name), "ts") => name.to_lower_camel_case(),
            (Some(name), "cs" | "java" | "kt") => name.to_upper_camel_case(),
            (Some(name), _) => name.to_snake_case(),
            (None, "py") => "__init__".to_owned(),
            (None, "rs") => "mod".to_owned(),
            (None, "cs" | "java" | "kt") => "Summary".to_owned(),
            (None, "ts") => "index".to_owned(),
            (None, "go") => "models".to_owned(),
            (None, _) => "summary".to_owned(),
        };

        let file_path = self.output_dir.join(format!("{basename}.{tpl_file_ext}"));
        let out_file = BufWriter::new(File::create(&file_path)?);

        self.tpl.render_to_write(ctx, out_file)?;

        if !self.no_postprocess {
            self.postprocessor.add_path(&file_path);
        }

        Ok(())
    }
}
