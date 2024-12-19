use std::{borrow::Cow, collections::BTreeMap, sync::Arc};

use aide::openapi::{self};
use anyhow::{bail, ensure, Context as _};
use schemars::schema::{InstanceType, Schema, SchemaObject, SingleOrVec};

use crate::util::get_schema_name;

/// Named types referenced by the [`Api`].
///
/// Intermediate representation of (some) `components` from the spec.
#[allow(dead_code)] // FIXME: Remove when we generate "model" files
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

    fn to_csharp_typename(&self) -> Cow<'_, str> {
        match self {
            Self::Bool => "bool".into(),
            // FIXME: For backwards compatibility. Should be 'long'.
            Self::UInt64 => "int".into(),
            Self::String => "string".into(),
            Self::DateTime => "DateTime".into(),
            Self::Set(field_type) => format!("List<{}>", field_type.to_csharp_typename()).into(),
            Self::SchemaRef(name) => name.clone().into(),
        }
    }

    fn to_go_typename(&self) -> Cow<'_, str> {
        match self {
            Self::Bool => "bool".into(),
            // FIXME: Looks like all integers are currently i32
            Self::UInt64 => "int32".into(),
            Self::String => "string".into(),
            Self::DateTime => "time.Time".into(),
            Self::Set(field_type) => format!("[]{}", field_type.to_go_typename()).into(),
            Self::SchemaRef(name) => name.clone().into(),
        }
    }

    fn to_kotlin_typename(&self) -> Cow<'_, str> {
        match self {
            Self::Bool => "Boolean".into(),
            Self::UInt64 => "Long".into(),
            Self::String => "String".into(),
            Self::DateTime => "OffsetDateTime".into(),
            Self::Set(field_type) => format!("List<{}>", field_type.to_kotlin_typename()).into(),
            Self::SchemaRef(name) => name.clone().into(),
        }
    }

    fn to_js_typename(&self) -> Cow<'_, str> {
        match self {
            Self::Bool => "boolean".into(),
            Self::UInt64 => "number".into(),
            Self::String => "string".into(),
            Self::DateTime => "Date | null".into(),
            Self::Set(field_type) => format!("{}[]", field_type.to_js_typename()).into(),
            Self::SchemaRef(name) => name.clone().into(),
        }
    }

    fn to_rust_typename(&self) -> Cow<'_, str> {
        match self {
            Self::Bool => "bool".into(),
            // FIXME: Looks like all integers are currently i32
            Self::UInt64 => "i32".into(),
            Self::String => "String".into(),
            // FIXME: Depends on those chrono imports being in scope, not that great..
            Self::DateTime => "DateTime<Utc>".into(),
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
            "to_csharp" => {
                ensure_no_args(args, "to_csharp")?;
                Ok(self.to_csharp_typename().into())
            }
            "to_go" => {
                ensure_no_args(args, "to_go")?;
                Ok(self.to_go_typename().into())
            }
            "to_js" => {
                ensure_no_args(args, "to_js")?;
                Ok(self.to_js_typename().into())
            }
            "to_kotlin" => {
                ensure_no_args(args, "to_kotlin")?;
                Ok(self.to_kotlin_typename().into())
            }
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
