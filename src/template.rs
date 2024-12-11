use std::borrow::Cow;

use heck::{ToSnakeCase as _, ToUpperCamelCase as _};

pub(crate) fn env() -> Result<minijinja::Environment<'static>, minijinja::Error> {
    let mut env = minijinja::Environment::new();

    // Custom filters
    env.add_filter("to_snake_case", |s: Cow<'_, str>| s.to_snake_case());
    env.add_filter("to_upper_camel_case", |s: Cow<'_, str>| {
        s.to_upper_camel_case()
    });

    // Templates
    env.add_template(
        "svix_resource",
        include_str!("../templates/svix_resource.rs.jinja"),
    )?;

    Ok(env)
}
