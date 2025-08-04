use std::collections::BTreeMap;

use fs_err as fs;
use serde::Serialize;

use crate::{
    api::{Api, Resource},
    types::{Type, Types},
    util::serialize_btree_map_values,
};

#[derive(Serialize)]
struct ApiAndTypes {
    #[serde(serialize_with = "serialize_btree_map_values")]
    pub resources: BTreeMap<String, Resource>,
    pub types: BTreeMap<String, Type>,
}

pub(crate) fn write_api_and_types(api: Api, types: Types) -> anyhow::Result<()> {
    let Api { resources } = api;
    let Types(types) = types;

    let api_and_types = ApiAndTypes { resources, types };
    let serialized = ron::ser::to_string_pretty(
        &api_and_types,
        ron::ser::PrettyConfig::new().extensions(ron::extensions::Extensions::IMPLICIT_SOME),
    )?;
    fs::write("debug.ron", serialized)?;

    Ok(())
}
