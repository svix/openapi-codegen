use std::{fs, io::BufWriter};

use anyhow::{bail, Context as _};
use camino::Utf8Path;
use fs_err::File;
use heck::{ToSnakeCase as _, ToUpperCamelCase as _};
use minijinja::{context, Template};
use serde::Deserialize;

use crate::{
    api::{Api, Resource},
    template,
    types::Types,
    util::run_postprocessing,
};

#[derive(Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum TemplateKind {
    #[default]
    ApiResource,
    ApiSummary,
    Type,
    TypeSummary,
}

pub(crate) fn generate(
    api: Api,
    types: Types,
    tpl_name: String,
    output_dir: &Utf8Path,
    no_postprocess: bool,
) -> anyhow::Result<()> {
    let (name_without_jinja_suffix, tpl_path) = match tpl_name.strip_suffix(".jinja") {
        Some(basename) => (basename, &tpl_name),
        None => (tpl_name.as_str(), &format!("{tpl_name}.jinja")),
    };

    let tpl_path_full = Utf8Path::new("templates").join(tpl_path);
    let (tpl_base_name, tpl_file_ext) = Utf8Path::new(name_without_jinja_suffix)
        .file_name()
        .context("template name must not end in '/'")?
        .rsplit_once(".")
        .context("template name must contain '.'")?;

    let tpl_kind = match tpl_base_name {
        "api_resource" => TemplateKind::ApiResource,
        "api_summary" => TemplateKind::ApiSummary,
        "component_type" => TemplateKind::Type,
        "component_type_summary" => TemplateKind::TypeSummary,
        _ => bail!(
            "template file basename must be one of 'api_resource', 'api_summary', \
             'component_type', 'component_type_summary'",
        ),
    };

    let tpl_source = fs::read_to_string(&tpl_path_full)?;

    let mut minijinja_env = template::env(
        tpl_path_full
            .parent()
            .with_context(|| format!("invalid template path `{tpl_path_full}`"))?,
    )?;
    minijinja_env.add_template(tpl_path, &tpl_source)?;
    let tpl = minijinja_env.get_template(tpl_path)?;

    let generator = Generator {
        tpl,
        tpl_file_ext,
        output_dir,
        no_postprocess,
    };

    match tpl_kind {
        TemplateKind::ApiResource => generator.generate_api_resources(api),
        TemplateKind::ApiSummary => generator.generate_api_summary(api),
        TemplateKind::Type => generator.generate_types(types),
        TemplateKind::TypeSummary => generator.generate_type_summary(types),
    }
}

struct Generator<'a> {
    tpl: Template<'a, 'a>,
    tpl_file_ext: &'a str,
    output_dir: &'a Utf8Path,
    no_postprocess: bool,
}

impl Generator<'_> {
    fn generate_api_resources(self, api: Api) -> anyhow::Result<()> {
        self.generate_api_resources_inner(api.resources.values())
    }

    fn generate_api_resources_inner<'a>(
        &self,
        resources: impl Iterator<Item = &'a Resource>,
    ) -> anyhow::Result<()> {
        for resource in resources {
            let referenced_components = resource.referenced_components();
            self.render_tpl(&resource.name, context! { resource, referenced_components })?;
            self.generate_api_resources_inner(resource.subresources.values())?;
        }

        Ok(())
    }

    fn generate_types(self, Types(types): Types) -> anyhow::Result<()> {
        for (name, ty) in types {
            let referenced_components = ty.referenced_components();
            self.render_tpl(&name, context! { type => ty, referenced_components })?;
        }

        Ok(())
    }

    fn generate_api_summary(&self, api: Api) -> anyhow::Result<()> {
        let name = match self.tpl_file_ext {
            "rs" => "mod",
            "py" => "__init__",
            _ => "summary",
        };
        self.render_tpl(name, context! { api })
    }

    fn generate_type_summary(&self, Types(types): Types) -> anyhow::Result<()> {
        let name = match self.tpl_file_ext {
            "rs" => "mod",
            "py" => "__init__",
            _ => "summary",
        };
        self.render_tpl(name, context! { types })
    }

    fn render_tpl(&self, output_name: &str, ctx: minijinja::Value) -> anyhow::Result<()> {
        let tpl_file_ext = self.tpl_file_ext;
        let basename = match tpl_file_ext {
            "cs" | "java" | "kt" => output_name.to_upper_camel_case(),
            "py" => {
                if output_name == "__init__" {
                    output_name.into()
                } else {
                    output_name.to_snake_case()
                }
            }
            _ => output_name.to_snake_case(),
        };

        let file_path = self.output_dir.join(format!("{basename}.{tpl_file_ext}"));
        let out_file = BufWriter::new(File::create(&file_path)?);

        self.tpl.render_to_write(ctx, out_file)?;
        if !self.no_postprocess {
            run_postprocessing(&file_path);
        }

        Ok(())
    }
}
