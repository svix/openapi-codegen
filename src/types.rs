use std::collections::BTreeMap;

use aide::openapi::{self};
use anyhow::{bail, ensure, Context as _};
use schemars::schema::{InstanceType, Schema, SchemaObject, SingleOrVec};

use crate::util::get_schema_name;

/// Named types referenced by the [`Api`].
///
/// Intermediate representation of (some) `components` from the spec.
#[derive(Debug)]
pub(crate) struct Types(pub BTreeMap<String, SchemaObject>);

/// Supported field type.
///
/// Equivalent to openapi's `type` + `format` + `$ref`.
#[derive(Debug, serde::Serialize)]
pub(crate) enum FieldType {
    Bool,
    UInt64,
    String,
    DateTime,
    /// List with unique items.
    Set(Box<FieldType>),
    SchemaRef(String),
}

impl FieldType {
    pub(crate) fn from_openapi(format: openapi::ParameterSchemaOrContent) -> anyhow::Result<Self> {
        let openapi::ParameterSchemaOrContent::Schema(s) = format else {
            bail!("found unexpected 'content' data format");
        };
        Self::from_json_schema(s.json_schema)
    }

    fn from_json_schema(s: Schema) -> anyhow::Result<Self> {
        let Schema::Object(obj) = s else {
            bail!("found unexpected `true` schema");
        };

        Ok(match obj.instance_type {
            Some(SingleOrVec::Single(ty)) => match *ty {
                InstanceType::Boolean => FieldType::Bool,
                InstanceType::Integer => match obj.format.as_deref() {
                    Some("uint64") => FieldType::UInt64,
                    f => bail!("unsupported integer format: `{f:?}`"),
                },
                InstanceType::String => match obj.format.as_deref() {
                    None => FieldType::String,
                    Some("date-time") => FieldType::DateTime,
                    Some(f) => bail!("unsupported string format: `{f:?}`"),
                },
                InstanceType::Array => {
                    let array = obj.array.context("array type must have array props")?;
                    ensure!(array.additional_items.is_none(), "not supported");
                    ensure!(
                        array.unique_items == Some(true),
                        "non-setlike arrays not currently supported"
                    );
                    let inner = match array.items.context("array type must have items prop")? {
                        SingleOrVec::Single(ty) => ty,
                        SingleOrVec::Vec(types) => {
                            bail!("unsupported multi-typed array parameter: `{types:?}`")
                        }
                    };
                    FieldType::Set(Box::new(Self::from_json_schema(*inner)?))
                }
                ty => bail!("unsupported type: `{ty:?}`"),
            },
            Some(SingleOrVec::Vec(types)) => {
                bail!("unsupported multi-typed parameter: `{types:?}`")
            }
            None => match get_schema_name(obj.reference) {
                Some(name) => FieldType::SchemaRef(name),
                None => bail!("unsupported type-less parameter"),
            },
        })
    }
}
