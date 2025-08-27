use std::borrow::Cow;

use camino::Utf8Path;
use fs_err as fs;
use heck::{
    ToLowerCamelCase as _, ToShoutySnakeCase as _, ToSnakeCase as _, ToUpperCamelCase as _,
};
use itertools::Itertools as _;
use minijinja::{State, Value, path_loader, value::Kwargs};

pub(crate) fn env(tpl_dir: &Utf8Path) -> Result<minijinja::Environment<'static>, minijinja::Error> {
    let mut env = minijinja::Environment::new();
    env.set_loader(path_loader(tpl_dir));

    // === Custom filters ===

    // --- Case conversion ---
    env.add_filter("to_upper_snake_case", |s: Cow<'_, str>| {
        s.to_shouty_snake_case()
    });
    env.add_filter("to_snake_case", |s: Cow<'_, str>| s.to_snake_case());
    env.add_filter("to_lower_camel_case", |s: Cow<'_, str>| {
        s.to_lower_camel_case()
    });
    env.add_filter("to_upper_camel_case", |s: Cow<'_, str>| {
        s.to_upper_camel_case()
    });

    // --- OpenAPI utils ---
    env.add_filter(
        "has_query_or_header_params",
        |operation: Value| -> Result<bool, minijinja::Error> {
            let query_params = operation.get_attr("query_params")?;
            let header_params = operation.get_attr("header_params")?;
            Ok(query_params.len() > Some(0) || header_params.len() > Some(0))
        },
    );
    env.add_filter(
        "has_required_query_or_header_params",
        |operation: Value| -> Result<bool, minijinja::Error> {
            let query_params = operation.get_attr("query_params")?;
            let header_params = operation.get_attr("header_params")?;
            Ok(contains_required_param(query_params)? || contains_required_param(header_params)?)
        },
    );
    env.add_filter(
        "has_non_ref_struct_enum_variants",
        |variants: Vec<Value>| -> Result<bool, minijinja::Error> {
            for v in &variants {
                let ty = v.get_attr("type")?;
                if ty.as_str().unwrap_or_default() != "ref" {
                    return Ok(true);
                }
            }
            Ok(false)
        },
    );

    // --- Comment generation ---
    env.add_filter(
        "to_doc_comment",
        |s: Cow<'_, str>, kwargs: Kwargs| -> Result<String, minijinja::Error> {
            let style: Cow<'_, str> = kwargs.get("style")?;
            kwargs.assert_all_used()?;

            let prefix = match &*style {
                "php_field" => {
                    if !s.contains("\n") {
                        return Ok(s.into());
                    } else {
                        return Ok(s.replace("\n", "\n    * "));
                    }
                }
                "python" => {
                    return Ok(format!(r#""""{s}""""#));
                }
                "java" | "kotlin" | "javascript" | "js" | "ts" | "typescript" | "php_class" => {
                    if !s.contains("\n") {
                        return Ok(format!("/** {s} */"));
                    }
                    let lines = s
                        .lines()
                        .format_with("\n", |line, f| f(&format_args!("* {line}")));
                    return Ok(format!("/**\n{lines}\n*/"));
                }
                "rust" | "csharp" => "///",
                "go" => "//",
                "ruby" => "#",
                _ => {
                    return Err(minijinja::Error::new(
                        minijinja::ErrorKind::UndefinedError,
                        "unsupported doc comment style",
                    ));
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

    // --- Miscellaneous ---
    env.add_filter("strip_trailing_comma", |s: Cow<'_, str>| {
        match s.trim_end().strip_suffix(",") {
            Some(stripped) => stripped.to_string(),
            None => s.into_owned(),
        }
    });

    env.add_filter(
        "generate_kt_path_str",
        |s: Cow<'_, str>, path_params: &Vec<Value>| -> Result<String, minijinja::Error> {
            let mut path_str = s.to_string();
            for field in path_params {
                let field = field.as_str().expect("Expected this to be a string");
                path_str = path_str.replace(
                    &format!("{{{field}}}"),
                    &format!("${}", field.to_lower_camel_case()),
                );
            }
            Ok(path_str)
        },
    );
    env.add_filter(
        "generate_java_path_str",
        |s: Cow<'_, str>, path_params: &Vec<Value>| -> Result<String, minijinja::Error> {
            let mut path_str = s.to_string();
            for field in path_params {
                let field = field.as_str().expect("Expected this to be a string");
                path_str = path_str.replace(&format!("{{{field}}}"), "%s");
            }
            Ok(path_str)
        },
    );
    env.add_filter(
        "generate_ruby_path_str",
        |s: Cow<'_, str>, path_params: &Vec<Value>| -> Result<String, minijinja::Error> {
            let mut path_str = s.to_string();
            for field in path_params {
                let field = field.as_str().expect("Expected this to be a string");
                path_str = path_str.replace(
                    &format!("{{{field}}}"),
                    &format!("#{{{}}}", field.to_snake_case()),
                );
            }
            Ok(path_str)
        },
    );
    env.add_filter(
        "generate_php_path_str",
        |s: Cow<'_, str>, path_params: &Vec<Value>| -> Result<String, minijinja::Error> {
            let mut path_str = s.to_string();
            for field in path_params {
                let field = field.as_str().expect("Expected this to be a string");
                path_str = path_str.replace(
                    &format!("{{{field}}}"),
                    &format!("{{${}}}", field.to_lower_camel_case()),
                );
            }
            Ok(path_str)
        },
    );

    env.add_function(
        // For java lib we need to create extra files.
        "generate_extra_file",
        |state: &State, filename: Cow<'_, str>, file_contents: Cow<'_, str>| {
            fs::write(&*filename, file_contents.as_bytes()).unwrap();
            state.set_temp("extra_generated_file", filename.into());
        },
    );

    env.add_filter(
        "struct_enum_ref_variants",
        |variants: Vec<Value>| -> Result<Vec<Value>, minijinja::Error> {
            variants
                .into_iter()
                .filter_map(|v| match v.get_attr("type") {
                    Ok(t) if t.as_str().unwrap_or_default() == "ref" => Some(Ok(v)),
                    Ok(_) => None,
                    Err(e) => Some(Err(e)),
                })
                .collect()
        },
    );

    env.add_filter(
        "struct_enum_struct_variants",
        |variants: Vec<Value>| -> Result<Vec<Value>, minijinja::Error> {
            variants
                .into_iter()
                .filter_map(|v| match v.get_attr("type") {
                    Ok(t) if t.as_str().unwrap_or_default() == "struct" => Some(Ok(v)),
                    Ok(_) => None,
                    Err(e) => Some(Err(e)),
                })
                .collect()
        },
    );

    Ok(env)
}

fn contains_required_param(value: Value) -> Result<bool, minijinja::Error> {
    for p in value.try_iter()? {
        if p.get_attr("required")?.is_true() {
            return Ok(true);
        }
    }

    Ok(false)
}
