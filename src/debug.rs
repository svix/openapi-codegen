use std::{
    collections::BTreeMap,
    io::{BufWriter, Write as _},
};

use fs_err::File;
use serde::Serialize;

use crate::{
    api::{Api, Resource},
    types::{Type, Types},
    util::serialize_btree_map_values,
};

#[derive(Debug, Serialize)]
struct ApiAndTypes {
    #[serde(serialize_with = "serialize_btree_map_values")]
    pub resources: BTreeMap<String, Resource>,
    pub types: BTreeMap<String, Type>,
}

pub(crate) fn write_api_and_types(api: Api, types: Types) -> anyhow::Result<()> {
    let Api { resources } = api;
    let Types(types) = types;

    let api_and_types = ApiAndTypes { resources, types };

    let mut output_file = BufWriter::new(File::create("debug.ron")?);
    writeln!(output_file, "{api_and_types:#?}")?;

    Ok(())
}
