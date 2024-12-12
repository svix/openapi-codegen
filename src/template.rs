use std::borrow::Cow;

use heck::{ToSnakeCase as _, ToUpperCamelCase as _};
use minijinja::value::Kwargs;

pub(crate) fn env() -> Result<minijinja::Environment<'static>, minijinja::Error> {
    let mut env = minijinja::Environment::new();

    // Perfect for Rust, maybe good enough for other langs too?
    env.set_formatter(|out, _state, value| {
        // FIXME: Is this a good idea?
        let s = value.to_string();
        write!(out, "{}", s.escape_default())?;
        Ok(())
    });

    // Custom filters
    env.add_filter("to_snake_case", |s: Cow<'_, str>| s.to_snake_case());
    env.add_filter("to_upper_camel_case", |s: Cow<'_, str>| {
        s.to_upper_camel_case()
    });
    #[allow(clippy::format_collect)] // suggestion more efficient, but harder to read
    env.add_filter(
        "to_doc_comment",
        |s: Cow<'_, str>, kwargs: Kwargs| -> Result<String, minijinja::Error> {
            let style: Cow<'_, str> = kwargs.get("style")?;
            kwargs.assert_all_used()?;

            let prefix = match &*style {
                "rust" | "javascript" | "js" | "ts" | "typescript" => "///",
                _ => {
                    return Err(minijinja::Error::new(
                        minijinja::ErrorKind::UndefinedError,
                        "unsupported doc comment style",
                    ))
                }
            };

            Ok(s.lines().map(|line| format!("{prefix} {line}")).collect())
        },
    );

    // Templates
    env.add_template(
        "svix_lib_resource",
        include_str!("../templates/svix_lib_resource.rs.jinja"),
    )?;
    env.add_template(
        "svix_cli_resource",
        include_str!("../templates/svix_cli_resource.rs.jinja"),
    )?;
    env.add_template(
        "svix_cli_types",
        include_str!("../templates/svix_cli_types.rs.jinja"),
    )?;

    Ok(env)
}
