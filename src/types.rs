use std::{borrow::Cow, collections::BTreeMap, sync::Arc};

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
#[derive(Clone, Debug)]
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
                InstanceType::Boolean => Self::Bool,
                InstanceType::Integer => match obj.format.as_deref() {
                    Some("uint64") => Self::UInt64,
                    f => bail!("unsupported integer format: `{f:?}`"),
                },
                InstanceType::String => match obj.format.as_deref() {
                    None => Self::String,
                    Some("date-time") => Self::DateTime,
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
                    Self::Set(Box::new(Self::from_json_schema(*inner)?))
                }
                ty => bail!("unsupported type: `{ty:?}`"),
            },
            Some(SingleOrVec::Vec(types)) => {
                bail!("unsupported multi-typed parameter: `{types:?}`")
            }
            None => match get_schema_name(obj.reference) {
                Some(name) => Self::SchemaRef(name),
                None => bail!("unsupported type-less parameter"),
            },
        })
    }

    fn to_rust_typename(&self) -> Cow<'_, str> {
        match self {
            Self::Bool => "bool".into(),
            Self::UInt64 => "u64".into(),
            Self::String => "String".into(),
            // FIXME: Use a better type
            Self::DateTime => "String".into(),
            // FIXME: Use BTreeSet
            Self::Set(field_type) => format!("Vec<{}>", field_type.to_rust_typename()).into(),
            Self::SchemaRef(name) => name.clone().into(),
        }
    }
}

impl minijinja::value::Object for FieldType {
    fn repr(self: &Arc<Self>) -> minijinja::value::ObjectRepr {
        minijinja::value::ObjectRepr::Plain
    }

    fn call_method(
        self: &Arc<Self>,
        _state: &minijinja::State<'_, '_>,
        method: &str,
        args: &[minijinja::Value],
    ) -> Result<minijinja::Value, minijinja::Error> {
        match method {
            "to_rust" => {
                ensure_no_args(args, "to_rust")?;
                Ok(self.to_rust_typename().into())
            }
            "is_datetime" => {
                ensure_no_args(args, "is_datetime")?;
                Ok(matches!(**self, Self::DateTime).into())
            }
            _ => Err(minijinja::Error::from(minijinja::ErrorKind::UnknownMethod)),
        }
    }
}

fn ensure_no_args(args: &[minijinja::Value], method_name: &str) -> Result<(), minijinja::Error> {
    if !args.is_empty() {
        return Err(minijinja::Error::new(
            minijinja::ErrorKind::TooManyArguments,
            format!("{method_name} does not take any arguments"),
        ));
    }
    Ok(())
}

impl serde::Serialize for FieldType {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        minijinja::Value::from_object(self.clone()).serialize(serializer)
    }
}
