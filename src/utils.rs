use std::sync::LazyLock;

use anyhow::Context as _;

use crate::{JsonObject, JsonValue};

pub(crate) fn get_properties(obj: &JsonValue) -> anyhow::Result<&JsonObject> {
    static EMPTY_OBJECT: LazyLock<JsonObject> = LazyLock::new(JsonObject::new);

    match obj.get("properties") {
        Some(v) => v.as_object().context("properties must be an object"),
        None => Ok(&EMPTY_OBJECT),
    }
}
