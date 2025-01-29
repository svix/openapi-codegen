use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

use serde::ser::{Serialize, SerializeSeq as _, Serializer};

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

pub(crate) fn serialize_btree_map_values<K, V, S>(
    map: &BTreeMap<K, V>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    V: Serialize,
    S: Serializer,
{
    let mut seq = serializer.serialize_seq(Some(map.len()))?;
    for item in map.values() {
        seq.serialize_element(item)?;
    }
    seq.end()
}

pub(crate) fn sha256sum_string(s: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(s.as_bytes());
    let hash = hasher.finalize();
    format!("{hash:x}")
}
