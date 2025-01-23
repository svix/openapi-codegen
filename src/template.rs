use std::borrow::Cow;

use camino::Utf8Path;
use heck::{ToLowerCamelCase as _, ToSnakeCase as _, ToUpperCamelCase as _};
use itertools::Itertools as _;
use minijinja::{path_loader, value::Kwargs};

pub(crate) fn env(tpl_dir: &Utf8Path) -> Result<minijinja::Environment<'static>, minijinja::Error> {
    let mut env = minijinja::Environment::new();
    env.set_loader(path_loader(tpl_dir));

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
                "python" => {
                    return Ok(format!(r#""""{s}""""#));
                }
                "java" | "kotlin" | "javascript" | "js" | "ts" | "typescript" => {
                    if !s.contains("\n") {
                        return Ok(format!("/** {s} */"));
                    }
                    let lines = s
                        .lines()
                        .format_with("\n", |line, f| f(&format_args!("* {line}")));
                    return Ok(format!("/**\n{lines}\n*/"));
                }
                "rust" => "///",
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
    env.add_filter(
        "with_javadoc_deprecation",
        |s: Cow<'_, str>, deprecated: bool| {
            if deprecated {
                if s.is_empty() {
                    "@deprecated".to_owned()
                } else {
                    s.into_owned() + "\n\n@deprecated"
                }
            } else {
                s.into_owned()
            }
        },
    );
    Ok(env)
}
