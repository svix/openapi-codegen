use std::io::{BufReader, BufWriter, Read};

use anyhow::Context as _;
use camino::Utf8Path;
use fs_err::File;
use heck::{ToSnakeCase as _, ToUpperCamelCase as _};
use minijinja::{context, Template};
use serde::Deserialize;

use crate::{
    api::{Api, Resource},
    template,
    types::Types,
    util::{parse_frontmatter, run_postprocessing},
};

#[derive(Default, Deserialize)]
struct TemplateFrontmatter {
    #[serde(default)]
    template_kind: TemplateKind,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum TemplateKind {
    #[default]
    ApiResource,
    ApiSummary,
    Type,
}

pub(crate) fn generate(
    api: Api,
    types: Types,
    tpl_name: String,
    output_dir: &Utf8Path,
    no_postprocess: bool,
) -> anyhow::Result<()> {
    let (tpl_file_ext, tpl_filename) = match tpl_name.strip_suffix(".jinja") {
        Some(basename) => (extension(basename), &tpl_name),
        None => (extension(&tpl_name), &format!("{tpl_name}.jinja")),
    };

    let tpl_file_ext = tpl_file_ext.context("template must have a file extension")?;
    let mut tpl_file = BufReader::new(File::open(Utf8Path::new("templates").join(tpl_filename))?);

    let tpl_frontmatter: TemplateFrontmatter = parse_frontmatter(&mut tpl_file)?;
    let mut tpl_source = String::new();
    tpl_file.read_to_string(&mut tpl_source)?;

    let mut minijinja_env = template::env()?;
    minijinja_env.add_template(tpl_filename, &tpl_source)?;
    let tpl = minijinja_env.get_template(tpl_filename)?;

    let generator = Generator {
        tpl,
        tpl_file_ext,
        output_dir,
        no_postprocess,
    };

    match tpl_frontmatter.template_kind {
        TemplateKind::ApiResource => generator.generate_api_resources(api),
        TemplateKind::ApiSummary => generator.generate_api_summary(api),
        TemplateKind::Type => generator.generate_types(types),
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

    fn generate_api_summary(&self, api: Api) -> anyhow::Result<()> {
        let name = match self.tpl_file_ext {
            "rs" => "mod",
            "py" => "__init__",
            _ => "summary",
        };
        self.render_tpl(name, context! { api })
    }

    fn generate_types(self, Types(types): Types) -> anyhow::Result<()> {
        for (name, ty) in types {
            let referenced_components = ty.referenced_components();
            self.render_tpl(&name, context! { type => ty, referenced_components })?;
        }

        Ok(())
    }

    fn render_tpl(&self, output_name: &str, ctx: minijinja::Value) -> anyhow::Result<()> {
        let tpl_file_ext = self.tpl_file_ext;
        let basename = match tpl_file_ext {
            "cs" | "java" | "kt" => output_name.to_upper_camel_case(),
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

fn extension(filename: &str) -> Option<&str> {
    Utf8Path::new(filename).extension()
}
