use std::io::BufWriter;

use anyhow::Context as _;
use camino::Utf8Path;
use fs_err::{self as fs, File};
use heck::{ToSnakeCase as _, ToUpperCamelCase as _};
use minijinja::context;

use crate::{api::Api, template, util::run_formatter};

pub(crate) fn generate_api(
    api: Api,
    tpl_name: String,
    output_dir: &Utf8Path,
    no_format: bool,
) -> anyhow::Result<()> {
    let (tpl_file_ext, tpl_filename) = match tpl_name.strip_suffix(".jinja") {
        Some(basename) => (extension(basename), &tpl_name),
        None => (extension(&tpl_name), &format!("{tpl_name}.jinja")),
    };

    let tpl_file_ext = tpl_file_ext.context("template must have a file extension")?;
    let tpl_source = fs::read_to_string(Utf8Path::new("templates").join(tpl_filename))?;

    let mut minijinja_env = template::env()?;
    minijinja_env.add_template(tpl_filename, &tpl_source)?;
    let tpl = minijinja_env.get_template(tpl_filename)?;

    for (name, resource) in api.resources {
        let basename = match tpl_file_ext {
            "cs" | "java" | "kt" => name.to_upper_camel_case(),
            _ => name.to_snake_case(),
        };

        let referenced_components = resource.referenced_components();
        let ctx = context! { resource, referenced_components };

        let file_path = output_dir.join(format!("{basename}.{tpl_file_ext}"));
        let out_file = BufWriter::new(File::create(&file_path)?);
        tpl.render_to_write(ctx, out_file)?;

        if !no_format {
            run_formatter(&file_path);
        }
    }

    Ok(())
}

fn extension(filename: &str) -> Option<&str> {
    Utf8Path::new(filename).extension()
}
