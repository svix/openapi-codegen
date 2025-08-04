use std::collections::BTreeSet;

mod resources;
mod types;

use aide::openapi;

use crate::{util::serialize_btree_map_values, IncludeMode};

pub(crate) use self::{
    resources::{Resource, Resources},
    types::Types,
};

#[derive(serde::Serialize)]
pub(crate) struct Api {
    #[serde(serialize_with = "serialize_btree_map_values")]
    pub resources: Resources,
    pub types: Types,
}

impl Api {
    pub(crate) fn new(
        paths: openapi::Paths,
        components: &mut openapi::Components,
        webhooks: &[String],
        include_mode: IncludeMode,
        excluded_operations: BTreeSet<String>,
        specified_operations: BTreeSet<String>,
    ) -> anyhow::Result<Self> {
        let resources = resources::from_openapi(
            paths,
            &components.schemas,
            include_mode,
            excluded_operations,
            specified_operations,
        )?;
        let types = types::from_referenced_components(
            &resources,
            &mut components.schemas,
            webhooks,
            include_mode,
        );

        Ok(Self { resources, types })
    }
}
