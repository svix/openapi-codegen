use std::collections::{BTreeSet, btree_map};

mod resources;
mod types;

use aide::openapi;
use serde::{Deserialize, Serialize};

use crate::IncludeMode;

pub(crate) use self::{
    resources::{Resource, Resources},
    types::Types,
};

#[derive(Default, Deserialize, Serialize)]
pub(crate) struct Api {
    #[serde(with = "toplevel_resources_serde")]
    pub resources: Resources,
    pub types: Types,
}

impl Api {
    pub(crate) fn new(
        paths: openapi::Paths,
        components: &mut openapi::Components,
        webhooks: &[String],
        include_mode: IncludeMode,
        excluded_operations: &BTreeSet<String>,
        specified_operations: &BTreeSet<String>,
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

impl FromIterator<Api> for Api {
    fn from_iter<T: IntoIterator<Item = Api>>(iter: T) -> Self {
        let mut api = Api::default();
        for item in iter {
            merge_resources(&mut api.resources, item.resources);
            api.types.extend(item.types);
        }
        api
    }
}

fn merge_resources(dst: &mut Resources, src: Resources) {
    for (k, v) in src {
        match dst.entry(k) {
            btree_map::Entry::Vacant(entry) => {
                entry.insert(v);
            }
            btree_map::Entry::Occupied(mut entry) => {
                let Resource {
                    name,
                    operations,
                    subresources,
                } = v;
                let dst_r = entry.get_mut();

                // Resource name must be equal to the dot-separated key path to
                // the resource. This invariant can be violated by hand-editing
                // the .ron file, but it's fine to crash in that case.
                assert_eq!(name, dst_r.name, "invalid resource name");

                dst_r.operations.extend(operations);
                merge_resources(&mut dst_r.subresources, subresources);
            }
        }
    }
}

pub(crate) fn get_schema_name(maybe_ref: Option<&str>) -> Option<String> {
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

pub(crate) mod toplevel_resources_serde {
    use std::fmt;

    use serde::{
        de::{Deserializer, SeqAccess, Visitor},
        ser::{SerializeSeq as _, Serializer},
    };

    use super::{Resource, Resources};

    pub(crate) fn serialize<S>(map: &Resources, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut seq = serializer.serialize_seq(Some(map.len()))?;
        for item in map.values() {
            seq.serialize_element(item)?;
        }
        seq.end()
    }

    pub(crate) fn deserialize<'de, D>(deserializer: D) -> Result<Resources, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct ToplevelResourcesVisitor;

        impl<'de> Visitor<'de> for ToplevelResourcesVisitor {
            type Value = Resources;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a list of resources")
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut resources = Resources::new();
                while let Some(r) = seq.next_element::<Resource>()? {
                    resources.insert(r.name.clone(), r);
                }
                Ok(resources)
            }
        }

        deserializer.deserialize_seq(ToplevelResourcesVisitor)
    }
}
