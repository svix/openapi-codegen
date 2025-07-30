use std::mem;

use aide::openapi::{Components, OpenApi, Operation, ParameterSchemaOrContent, ReferenceOr};
use indexmap::IndexMap;
use schemars::schema::{Schema, SingleOrVec};

use crate::util::prefix_op_id;

/// Add `ee_` prefix to all schema and operation names
pub fn add_ee_prefix(spec: &mut OpenApi) {
    spec.components.as_mut().map(add_prefix_to_components);

    if let Some(paths) = spec.paths.as_mut() {
        for p in paths.paths.as_mut_slice() {
            let path = p.1.as_item_mut().unwrap();
            path.post.as_mut().map(add_prefix_to_op);
            path.get.as_mut().map(add_prefix_to_op);
            path.put.as_mut().map(add_prefix_to_op);
            path.patch.as_mut().map(add_prefix_to_op);
            path.head.as_mut().map(add_prefix_to_op);
            path.options.as_mut().map(add_prefix_to_op);
            path.trace.as_mut().map(add_prefix_to_op);
        }
    }
}

fn add_prefix_to_components(components: &mut Components) {
    rename_keys(&mut components.schemas, |s| prefix_str(s));

    for v in components.schemas.values_mut() {
        add_prefix_to_schema(&mut v.json_schema);
    }
}

fn add_prefix_to_schema(json_schema: &mut Schema) {
    match json_schema {
        Schema::Bool(_) => (),
        Schema::Object(schema_object) => add_prefix_to_schema_obj(schema_object),
    }
}
fn add_prefix_to_schema_obj(schema_object: &mut schemars::schema::SchemaObject) {
    if let Some(r) = schema_object.reference.as_mut() {
        prefix_ref_in_place(r)
    }

    if let Some(obj) = schema_object.object.as_mut() {
        for v in obj.properties.values_mut() {
            add_prefix_to_schema(v);
        }
    }

    if let Some(array) = schema_object.array.as_mut() {
        if let Some(items) = array.items.as_mut() {
            match items {
                SingleOrVec::Single(item) => {
                    add_prefix_to_schema(item);
                }
                SingleOrVec::Vec(items) => {
                    let _ = items.iter_mut().map(add_prefix_to_schema);
                }
            }
        }
    }
}

fn add_prefix_to_op(op: &mut Operation) {
    if let Some(op_id) = op.operation_id.as_mut() {
        let prefixed_op_id = prefix_op_id(op_id);
        *op_id = prefixed_op_id;
    }

    if let Some(body) = op.request_body.as_mut() {
        match body {
            ReferenceOr::Reference { reference, .. } => prefix_ref_in_place(reference),
            ReferenceOr::Item(body) => {
                for v in body.content.values_mut() {
                    if let Some(v) = v.schema.as_mut() {
                        add_prefix_to_schema(&mut v.json_schema)
                    }
                }
            }
        }
    }

    if let Some(r) = op.responses.as_mut() {
        for res in r.responses.values_mut() {
            match res {
                ReferenceOr::Reference { reference, .. } => prefix_ref_in_place(reference),
                ReferenceOr::Item(body) => {
                    for v in body.content.values_mut() {
                        if let Some(v) = v.schema.as_mut() {
                            add_prefix_to_schema(&mut v.json_schema)
                        }
                    }
                }
            }
        }
    }

    for param in op.parameters.iter_mut() {
        match param {
            ReferenceOr::Reference { reference, .. } => prefix_ref_in_place(reference),
            ReferenceOr::Item(item) => {
                let param_data = item.parameter_data_mut();
                match &mut param_data.format {
                    ParameterSchemaOrContent::Schema(schema_object) => {
                        add_prefix_to_schema(&mut schema_object.json_schema)
                    }
                    ParameterSchemaOrContent::Content(index_map) => {
                        for v in index_map.values_mut() {
                            if let Some(v) = v.schema.as_mut() {
                                add_prefix_to_schema(&mut v.json_schema)
                            }
                        }
                    }
                }
            }
        }
    }
}

fn rename_keys<K, V, F>(map: &mut IndexMap<K, V>, mut f: F)
where
    K: std::hash::Hash + Eq,
    F: FnMut(&K) -> K,
{
    let mut new_map = IndexMap::with_capacity(map.len());

    for (old_key, value) in map.drain(..) {
        let new_key = f(&old_key);
        new_map.insert(new_key, value);
    }

    *map = new_map;
}

fn prefix_str<T: AsRef<str>>(v: T) -> String {
    format!("Ee{}", v.as_ref())
}

// apply ee prefix to $ref strings
fn prefix_ref<T: AsRef<str>>(v: T) -> String {
    v.as_ref()
        .replace("#/components/schemas/", "#/components/schemas/Ee")
}

// apply ee prefix *in-place* to $ref strings
fn prefix_ref_in_place(v: &mut String) {
    let r = mem::take(v);
    *v = prefix_ref(r);
}
