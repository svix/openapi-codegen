use std::borrow::Cow;

use heck::{ToLowerCamelCase as _, ToSnakeCase as _, ToUpperCamelCase as _};
use itertools::Itertools as _;
use minijinja::{path_loader, value::Kwargs};

pub(crate) fn env() -> Result<minijinja::Environment<'static>, minijinja::Error> {
    let mut env = minijinja::Environment::new();
    env.set_loader({
        let loader = path_loader("templates");
        // Try finding the specified template
        move |name| match loader(name)? {
            Some(src) => Ok(Some(src)),
            // If not found, try with extra `.jinja` extension
            None => loader(&format!("{name}.jinja")),
        }
    });

    // Custom filters
    env.add_filter("to_snake_case", |s: Cow<'_, str>| s.to_snake_case());
    env.add_filter("to_lower_camel_case", |s: Cow<'_, str>| {
        s.to_lower_camel_case()
    });
    env.add_filter("to_upper_camel_case", |s: Cow<'_, str>| {
        s.to_upper_camel_case()
    });
    env.add_filter(
        "to_doc_comment",
        |s: Cow<'_, str>, kwargs: Kwargs| -> Result<String, minijinja::Error> {
            let style: Cow<'_, str> = kwargs.get("style")?;
            kwargs.assert_all_used()?;

            let prefix = match &*style {
                "rust" | "javascript" | "js" | "ts" | "typescript" => "///",
                "go" => "//",
                _ => {
                    return Err(minijinja::Error::new(
                        minijinja::ErrorKind::UndefinedError,
                        "unsupported doc comment style",
                    ))
                }
            };

            Ok(s.lines()
                .format_with("\n", |line, f| f(&format_args!("{prefix} {line}")))
                .to_string())
        },
    );

    Ok(env)
}
