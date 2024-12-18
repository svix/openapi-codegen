use camino::Utf8Path;

pub(crate) fn get_schema_name(maybe_ref: Option<String>) -> Option<String> {
    let r = maybe_ref?;
    let schema_name = r.strip_prefix("#/components/schemas/");
    if schema_name.is_none() {
        tracing::warn!(
            component_ref = r,
            "missing #/components/schemas/ prefix on component ref"
        );
    };
    Some(schema_name?.to_owned())
}

pub(crate) fn run_formatter(path: &Utf8Path) {
    let Some(file_ext) = path.extension() else {
        return;
    };

    if file_ext == "rs" {
        _ = std::process::Command::new("rustfmt")
            .args(["+nightly", "--edition", "2021"])
            .arg(path)
            .status();
    }
}
