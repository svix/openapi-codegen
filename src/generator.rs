use std::io::BufWriter;

use anyhow::Context as _;
use camino::Utf8Path;
use fs_err::File;
use heck::{ToSnakeCase as _, ToUpperCamelCase as _};
use minijinja::context;

use crate::{api::Api, template, util::run_formatter};

pub(crate) fn generate_api(
    api: Api,
    template_name: &str,
    output_dir: &Utf8Path,
    no_format: bool,
) -> anyhow::Result<()> {
    // Use the second `.`-separated segment of the filename, so for
    // `foo.rs.jinja` this get us `rs`, not `jinja`.
    let tpl_file_ext = template_name
        .split('.')
        .nth(1)
        .context("template must have a file extension")?;

    let minijinja_env = template::env()?;
    let tpl = minijinja_env.get_template(template_name)?;

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
