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
