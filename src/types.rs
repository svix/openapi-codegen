use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use aide::openapi;
use anyhow::{bail, ensure, Context as _};
use indexmap::IndexMap;
use schemars::schema::{InstanceType, ObjectValidation, Schema, SchemaObject, SingleOrVec};
use serde::Serialize;

use crate::util::get_schema_name;

/// Named types referenced by the [`Api`].
///
/// Intermediate representation of (some) `components` from the spec.
#[derive(Debug)]
pub(crate) struct Types(pub BTreeMap<String, Type>);

impl Types {
    pub(crate) fn from_referenced_components<'a>(
        schemas: &mut IndexMap<String, openapi::SchemaObject>,
        components: impl Iterator<Item = &'a str>,
    ) -> Self {
        let mut types = BTreeMap::new();
        let mut add_type = |schema_name: &str, extra_components: &mut BTreeSet<_>| {
            let Some(s) = schemas.swap_remove(schema_name) else {
                tracing::warn!(schema_name, "schema not found");
                return;
            };

            let obj = match s.json_schema {
                Schema::Bool(_) => {
                    tracing::warn!(schema_name, "found $ref'erenced bool schema, wat?!");
                    return;
                }
                Schema::Object(o) => o,
            };

            match Type::from_schema(schema_name.to_owned(), obj) {
                Ok(ty) => {
                    extra_components.extend(
                        ty.referenced_components()
                            .into_iter()
                            .filter(|&c| c != schema_name && !types.contains_key(c))
                            .map(ToOwned::to_owned),
                    );
                    types.insert(schema_name.to_owned(), ty);
                }
                Err(e) => {
                    tracing::warn!(schema_name, "unsupported schema: {e:#}");
                }
            }
        };

        let mut extra_components: BTreeSet<_> = components.map(ToOwned::to_owned).collect();
        while let Some(c) = extra_components.pop_first() {
            add_type(&c, &mut extra_components);
        }

        Self(types)
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct Type {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    deprecated: bool,
    #[serde(flatten)]
    data: TypeData,
}

impl Type {
    pub(crate) fn from_schema(name: String, s: SchemaObject) -> anyhow::Result<Self> {
        let data = match s.instance_type {
            Some(SingleOrVec::Single(it)) => match *it {
                InstanceType::Object => {
                    let obj = s
                        .object
                        .context("unsupported: object type without further validation")?;
                    TypeData::from_object_schema(*obj)?
                }
                InstanceType::Integer => {
                    let values = s
                        .enum_values
                        .context("unsupported: integer type without enum values")?;
                    TypeData::from_integer_enum(values)?
                }
                InstanceType::String => {
                    let values = s
                        .enum_values
                        .context("unsupported: string type without enum values")?;
                    TypeData::from_string_enum(values)?
                }
                _ => bail!("unsupported type {it:?}"),
            },
            Some(SingleOrVec::Vec(_)) => bail!("unsupported: multiple types"),
            None => bail!("unsupported: no type"),
        };

        let metadata = s.metadata.unwrap_or_default();

        Ok(Self {
            name,
            description: metadata.description,
            deprecated: metadata.deprecated,
            data,
        })
    }

    pub(crate) fn referenced_components(&self) -> BTreeSet<&str> {
        match &self.data {
            TypeData::Struct { fields } => fields
                .iter()
                .filter_map(|f| f.r#type.referenced_schema())
                .collect(),
            TypeData::StringEnum { .. } => BTreeSet::new(),
            TypeData::IntegerEnum { .. } => BTreeSet::new(),
            TypeData::StructEnum { variants } => variants
                .iter()
                .flat_map(|v| &v.fields)
                .filter_map(|f| f.r#type.referenced_schema())
                .collect(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum TypeData {
    Struct {
        fields: Vec<Field>,
    },
    StringEnum {
        values: Vec<String>,
    },
    IntegerEnum {
        values: Vec<i64>,
    },
    #[allow(dead_code)] // not _yet_ supported
    StructEnum {
        variants: Vec<Variant>,
    },
}

impl TypeData {
    fn from_object_schema(obj: ObjectValidation) -> anyhow::Result<Self> {
        ensure!(
            obj.additional_properties.is_none(),
            "additional_properties not yet supported"
        );
        ensure!(obj.max_properties.is_none(), "unsupported: max_properties");
        ensure!(obj.min_properties.is_none(), "unsupported: min_properties");
        ensure!(
            obj.pattern_properties.is_empty(),
            "unsupported: pattern_properties"
        );
        ensure!(obj.property_names.is_none(), "unsupported: property_names");

        Ok(Self::Struct {
            fields: obj
                .properties
                .into_iter()
                .map(|(name, schema)| {
                    Field::from_schema(name.clone(), schema, obj.required.contains(&name))
                        .with_context(|| format!("unsupported field {name}"))
                })
                .collect::<anyhow::Result<_>>()?,
        })
    }

    fn from_string_enum(values: Vec<serde_json::Value>) -> anyhow::Result<TypeData> {
        Ok(Self::StringEnum {
            values: values
                .into_iter()
                .enumerate()
                .map(|(i, v)| match v {
                    serde_json::Value::String(s) => Ok(s),
                    _ => bail!("enum value {} is not a string", i + 1),
                })
                .collect::<anyhow::Result<_>>()?,
        })
    }

    fn from_integer_enum(values: Vec<serde_json::Value>) -> anyhow::Result<TypeData> {
        Ok(Self::IntegerEnum {
            values: values
                .into_iter()
                .enumerate()
                .map(|(i, v)| match v {
                    serde_json::Value::Number(s) => s
                        .as_i64()
                        .with_context(|| format!("enum value {s} is not an integer")),
                    _ => bail!("enum value {} is not a number", i + 1),
                })
                .collect::<anyhow::Result<_>>()?,
        })
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct Field {
    name: String,
    r#type: FieldType,
    #[serde(skip_serializing_if = "Option::is_none")]
    default: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    required: bool,
    nullable: bool,
    deprecated: bool,
}

impl Field {
    fn from_schema(name: String, s: Schema, required: bool) -> anyhow::Result<Self> {
        let obj = match s {
            Schema::Bool(_) => bail!("unsupported bool schema"),
            Schema::Object(o) => o,
        };
        let metadata = obj.metadata.clone().unwrap_or_default();

        ensure!(obj.const_value.is_none(), "unsupported const_value");
        ensure!(obj.enum_values.is_none(), "unsupported enum_values");

        let nullable = obj
            .extensions
            .get("nullable")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        Ok(Self {
            name,
            r#type: FieldType::from_schema_object(obj)?,
            default: metadata.default,
            description: metadata.description,
            required,
            nullable,
            deprecated: metadata.deprecated,
        })
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct Variant {
    fields: Vec<Field>,
}

/// Supported field type.
///
/// Equivalent to openapi's `type` + `format` + `$ref`.
#[derive(Clone, Debug)]
pub(crate) enum FieldType {
    Bool,
    Int16,
    UInt16,
    Int32,
    Int64,
    UInt64,
    String,
    DateTime,
    Uri,
    /// A JSON object with arbitrary field values.
    JsonObject,
    /// A regular old list.
    List(Box<FieldType>),
    /// List with unique items.
    Set(Box<FieldType>),
    /// A map with a given value type.
    ///
    /// The key type is always `String` in JSON schemas.
    Map {
        value_ty: Box<FieldType>,
    },
    SchemaRef(String),
}

impl FieldType {
    pub(crate) fn from_openapi(format: openapi::ParameterSchemaOrContent) -> anyhow::Result<Self> {
        let openapi::ParameterSchemaOrContent::Schema(s) = format else {
            bail!("found unexpected 'content' data format");
        };
        Self::from_schema(s.json_schema)
    }

    fn from_schema(s: Schema) -> anyhow::Result<Self> {
        let Schema::Object(obj) = s else {
            bail!("found unexpected `true` schema");
        };

        Self::from_schema_object(obj)
    }

    fn from_schema_object(obj: SchemaObject) -> anyhow::Result<FieldType> {
        Ok(match &obj.instance_type {
            Some(SingleOrVec::Single(ty)) => match **ty {
                InstanceType::Boolean => Self::Bool,
                InstanceType::Integer => match obj.format.as_deref() {
                    Some("int16") => Self::Int16,
                    Some("uint16") => Self::UInt16,
                    Some("int32") => Self::Int32,
                    // FIXME: Why do we have int in the spec?
                    Some("int" | "int64") => Self::Int64,
                    // FIXME: Get rid of uint in the spec..
                    Some("uint" | "uint64") => Self::UInt64,
                    f => bail!("unsupported integer format: `{f:?}`"),
                },
                InstanceType::String => match obj.format.as_deref() {
                    None => Self::String,
                    Some("date-time") => Self::DateTime,
                    Some("uri") => Self::Uri,
                    Some(f) => bail!("unsupported string format: `{f:?}`"),
                },
                InstanceType::Array => {
                    let array = obj.array.context("array type must have array props")?;
                    ensure!(array.additional_items.is_none(), "not supported");
                    let inner = match array.items.context("array type must have items prop")? {
                        SingleOrVec::Single(ty) => ty,
                        SingleOrVec::Vec(types) => {
                            bail!("unsupported multi-typed array parameter: `{types:?}`")
                        }
                    };
                    let inner = Box::new(Self::from_schema(*inner)?);
                    if array.unique_items == Some(true) {
                        Self::Set(inner)
                    } else {
                        Self::List(inner)
                    }
                }
                InstanceType::Object => {
                    let obj = obj
                        .object
                        .context("unsupported: object type without further validation")?;
                    let additional_properties = obj
                        .additional_properties
                        .context("unsupported: object field type without additional_properties")?;

                    ensure!(obj.max_properties.is_none(), "unsupported: max_properties");
                    ensure!(obj.min_properties.is_none(), "unsupported: min_properties");
                    ensure!(
                        obj.properties.is_empty(),
                        "unsupported: properties on field type"
                    );
                    ensure!(
                        obj.pattern_properties.is_empty(),
                        "unsupported: pattern_properties"
                    );
                    ensure!(obj.property_names.is_none(), "unsupported: property_names");
                    ensure!(
                        obj.required.is_empty(),
                        "unsupported: required on field type"
                    );

                    match *additional_properties {
                        Schema::Bool(true) => Self::JsonObject,
                        Schema::Bool(false) => bail!("unsupported `additional_properties: false`"),
                        Schema::Object(schema_object) => {
                            let value_ty = Box::new(Self::from_schema_object(schema_object)?);
                            Self::Map { value_ty }
                        }
                    }
                }
                ty => bail!("unsupported type: `{ty:?}`"),
            },
            Some(SingleOrVec::Vec(types)) => {
                bail!("unsupported multi-typed parameter: `{types:?}`")
            }
            None => match get_schema_name(obj.reference.as_deref()) {
                Some(name) => Self::SchemaRef(name),
                None => bail!("unsupported type-less parameter"),
            },
        })
    }

    fn to_csharp_typename(&self) -> Cow<'_, str> {
        match self {
            Self::Bool => "bool".into(),
            Self::Int32 |
            // FIXME: For backwards compatibility. Should be 'long'.
            Self::Int64 | Self::UInt64 => "int".into(),
            Self::String => "string".into(),
            Self::DateTime => "DateTime".into(),
            Self::Int16 | Self::UInt16 | Self::Uri | Self::JsonObject | Self::Map { .. } => todo!(),
            // FIXME: Treat set differently?
            Self::List(field_type) | Self::Set(field_type) => {
                format!("List<{}>", field_type.to_csharp_typename()).into()
            }
            Self::SchemaRef(name) => name.clone().into(),
        }
    }

    fn to_go_typename(&self) -> Cow<'_, str> {
        match self {
            Self::Bool => "bool".into(),
            Self::Int32 |
            // FIXME: Looks like all integers are currently i32
            Self::Int64 | Self::UInt64 => "int32".into(),
            Self::String => "string".into(),
            Self::DateTime => "time.Time".into(),
            Self::Int16 | Self::UInt16 | Self::Uri | Self::JsonObject | Self::Map { .. } => todo!(),
            Self::List(field_type) | Self::Set(field_type) => {
                format!("[]{}", field_type.to_go_typename()).into()
            }
            Self::SchemaRef(name) => name.clone().into(),
        }
    }

    fn to_kotlin_typename(&self) -> Cow<'_, str> {
        match self {
            Self::Bool => "Boolean".into(),
            Self::Int32 |
            // FIXME: Should be Long..
            Self::Int64 | Self::UInt64 => "Int".into(),
            Self::String => "String".into(),
            Self::DateTime => "OffsetDateTime".into(),
            Self::Int16 | Self::UInt16 | Self::Uri | Self::JsonObject | Self::Map { .. } => todo!(),
            // FIXME: Treat set differently?
            Self::List(field_type) | Self::Set(field_type) => {
                format!("List<{}>", field_type.to_kotlin_typename()).into()
            }
            Self::SchemaRef(name) => name.clone().into(),
        }
    }

    fn to_js_typename(&self) -> Cow<'_, str> {
        match self {
            Self::Bool => "boolean".into(),
            Self::Int16 | Self::UInt16 | Self::Int32 | Self::Int64 | Self::UInt64 => {
                "number".into()
            }
            Self::String => "string".into(),
            Self::DateTime => "Date | null".into(),
            Self::Uri | Self::JsonObject | Self::Map { .. } => todo!(),
            Self::List(field_type) | Self::Set(field_type) => {
                format!("{}[]", field_type.to_js_typename()).into()
            }
            Self::SchemaRef(name) => name.clone().into(),
        }
    }

    fn to_rust_typename(&self) -> Cow<'_, str> {
        match self {
            Self::Bool => "bool".into(),
            Self::Int16 => "i16".into(),
            Self::UInt16 => "u16".into(),
            Self::Int32 |
            // FIXME: All integers in query params are currently i32
            Self::Int64 | Self::UInt64 => "i32".into(),
            // FIXME: Do we want a separate type for Uri?
            Self::Uri | Self::String => "String".into(),
            // FIXME: Depends on those chrono imports being in scope, not that great..
            Self::DateTime => "DateTime<Utc>".into(),
            Self::JsonObject => "serde_json::Value".into(),
            // FIXME: Treat set differently? (BTreeSet)
            Self::List(field_type) | Self::Set(field_type) => {
                format!("Vec<{}>", field_type.to_rust_typename()).into()
            }
            Self::Map { value_ty } => format!(
                "std::collections::HashMap<String, {}>",
                value_ty.to_rust_typename(),
            )
            .into(),
            Self::SchemaRef(name) => name.clone().into(),
        }
    }

    pub(crate) fn referenced_schema(&self) -> Option<&str> {
        match self {
            Self::SchemaRef(v) => Some(v),
            Self::List(ty) | Self::Set(ty) | Self::Map { value_ty: ty } => ty.referenced_schema(),
            _ => None,
        }
    }

    fn to_python_typename(&self) -> Cow<'_, str> {
        match self {
            Self::Bool => "bool".into(),
            Self::Int16 | Self::UInt16 | Self::Int32 | Self::Int64 | Self::UInt64 => "int".into(),
            Self::String => "str".into(),
            Self::DateTime => "datetime".into(),
            Self::Set(field_type) => format!("t.Set[{}]", field_type.to_python_typename()).into(),
            Self::SchemaRef(name) => name.clone().into(),
            Self::Uri => "str".into(),
            Self::JsonObject => "t.Dict[str, t.Any]".into(),
            Self::List(field_type) => format!("t.List[{}]", field_type.to_python_typename()).into(),
            Self::Map { value_ty } => {
                format!("t.Dict[str, {}]", value_ty.to_python_typename()).into()
            }
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
            "to_python" => {
                ensure_no_args(args, "to_python")?;
                Ok(self.to_python_typename().into())
            }
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
            "is_schema_ref" => {
                ensure_no_args(args, "is_datetime")?;
                Ok(matches!(**self, Self::SchemaRef(_)).into())
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
