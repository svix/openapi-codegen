use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use aide::openapi;
use anyhow::{bail, ensure, Context as _};
use indexmap::IndexMap;
use schemars::schema::{
    InstanceType, ObjectValidation, Schema, SchemaObject, SingleOrVec, SubschemaValidation,
};
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
                Ok(tys) => {
                    extra_components.extend(
                        tys.iter()
                            .flat_map(|ty| ty.referenced_components())
                            .filter(|&c| c != schema_name && !types.contains_key(c))
                            .map(ToOwned::to_owned),
                    );
                    types.extend(tys.into_iter().map(|ty| (ty.name.clone(), ty)));
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
    pub(crate) fn from_schema(name: String, s: SchemaObject) -> anyhow::Result<Vec<Self>> {
        let mut result = Vec::new();
        let data = match s.instance_type {
            Some(SingleOrVec::Single(it)) => match *it {
                InstanceType::Object => {
                    let obj = s
                        .object
                        .context("unsupported: object type without further validation")?;
                    let (data, aux_ty) = TypeData::from_object_schema(&name, *obj, s.subschemas)?;
                    result.extend(aux_ty);

                    data
                }
                InstanceType::Integer => {
                    let enum_varnames = s
                        .extensions
                        .get("x-enum-varnames")
                        .context("unsupported: integer type without enum varnames")?
                        .as_array()
                        .context("unsupported: integer type enum varnames should be a list")?;
                    let values = s
                        .enum_values
                        .context("unsupported: integer type without enum values")?;
                    if enum_varnames.len() != values.len() {
                        bail!(
                            "enum varnames length ({}) does not match values length ({})",
                            enum_varnames.len(),
                            values.len()
                        );
                    }
                    TypeData::from_integer_enum(values, enum_varnames)?
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

        result.push(Self {
            name,
            description: metadata.description,
            deprecated: metadata.deprecated,
            data,
        });
        Ok(result)
    }

    pub(crate) fn referenced_components(&self) -> BTreeSet<&str> {
        match &self.data {
            TypeData::Struct { fields } => fields
                .iter()
                .filter_map(|f| f.r#type.referenced_schema())
                .collect(),
            TypeData::StringEnum { .. } => BTreeSet::new(),
            TypeData::IntegerEnum { .. } => BTreeSet::new(),
            TypeData::StructEnum { variants, .. } => variants
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
        variants: Vec<(String, i64)>,
    },
    StructEnum {
        /// Name of the field that identifies the variant.
        discriminator: String,
        variants: Vec<Variant>,
    },
}

impl TypeData {
    fn from_object_schema(
        name: &str,
        obj: ObjectValidation,
        subschemas: Option<Box<SubschemaValidation>>,
    ) -> anyhow::Result<(Self, Option<Type>)> {
        ensure!(
            obj.additional_properties.is_none(),
            "additionalProperties not yet supported"
        );
        ensure!(obj.max_properties.is_none(), "unsupported: maxProperties");
        ensure!(obj.min_properties.is_none(), "unsupported: minProperties");
        ensure!(
            obj.pattern_properties.is_empty(),
            "unsupported: patternProperties"
        );
        ensure!(obj.property_names.is_none(), "unsupported: propertyNames");

        let mut fields: Vec<_> = obj
            .properties
            .into_iter()
            .map(|(name, schema)| {
                Field::from_schema(name.clone(), schema, obj.required.contains(&name))
                    .with_context(|| format!("unsupported field {name}"))
            })
            .collect::<anyhow::Result<_>>()?;

        let mut aux_ty = None;
        if let Some(sub) = subschemas {
            ensure!(sub.all_of.is_none(), "unsupported: allOf subschema");
            ensure!(sub.any_of.is_none(), "unsupported: anyOf subschema");
            ensure!(sub.not.is_none(), "unsupported: not subschema");
            ensure!(sub.if_schema.is_none(), "unsupported: if subschema");
            ensure!(sub.then_schema.is_none(), "unsupported: then subschema");
            ensure!(sub.else_schema.is_none(), "unsupported: else subschema");

            if let Some(one_of) = sub.one_of {
                let (discriminator, variants) = Self::struct_enum(one_of)?;
                let enum_data = Self::StructEnum {
                    discriminator: discriminator.clone(),
                    variants,
                };

                if fields.is_empty() {
                    return Ok((enum_data, None));
                } else {
                    let enum_name = format!("{name}_{discriminator}");

                    aux_ty = Some(Type {
                        name: enum_name.clone(),
                        description: None,
                        deprecated: false,
                        data: enum_data,
                    });

                    fields.push(Field {
                        name: discriminator,
                        r#type: FieldType::SchemaRef(enum_name),
                        default: None,
                        description: None,
                        required: true,
                        nullable: false,
                        deprecated: false,
                    });
                }
            }
        }

        Ok((Self::Struct { fields }, aux_ty))
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

    fn from_integer_enum(
        values: Vec<serde_json::Value>,
        enum_varnames: &[serde_json::Value],
    ) -> anyhow::Result<TypeData> {
        Ok(Self::IntegerEnum {
            variants: values
                .into_iter()
                .enumerate()
                .map(|(i, v)| match v {
                    serde_json::Value::Number(s) => {
                        let num = s
                            .as_i64()
                            .with_context(|| format!("enum value {s} is not an integer"))?;
                        Ok((
                            enum_varnames[i]
                                .as_str()
                                .context(format!(
                                    "enum varname {} is not a string",
                                    &enum_varnames[i]
                                ))?
                                .to_string(),
                            num,
                        ))
                    }
                    _ => bail!("enum value {} is not a number", i + 1),
                })
                .collect::<anyhow::Result<_>>()?,
        })
    }

    fn struct_enum(one_of: Vec<Schema>) -> anyhow::Result<(String, Vec<Variant>)> {
        let mut discriminator = None;

        let variants = one_of
            .into_iter()
            .map(|variant| {
                let Schema::Object(s) = variant else {
                    bail!("unsupported: boolean subschema");
                };

                let data = match s.instance_type {
                    Some(SingleOrVec::Single(it)) => match *it {
                        InstanceType::Object => {
                            let obj = s
                                .object
                                .context("unsupported: object type without further validation")?;
                            ensure!(s.subschemas.is_none(), "unsupported: nested subschemas");

                            let (data, aux_ty) = TypeData::from_object_schema("", *obj, None)?;
                            ensure!(
                                aux_ty.is_none(),
                                "unsupported: subschema that defines an auxiliary type"
                            );

                            data
                        }
                        _ => bail!("unsupported type in subschema: {it:?}"),
                    },
                    Some(SingleOrVec::Vec(_)) => {
                        bail!("unsupported: multiple types in subschema")
                    }
                    None => bail!("unsupported: no type"),
                };

                let TypeData::Struct { mut fields } = data else {
                    bail!("unsupported: oneOf schema with non-struct member(s)");
                };

                let mut name = None;

                fields.retain_mut(|f| match &f.r#type {
                    FieldType::StringConst(value) => {
                        if name.is_some() {
                            // would be nice to be able to bail, but can't from retain_mut
                            tracing::error!("found two names for one enum variant");
                            return false;
                        }

                        if let Some(d) = &discriminator {
                            if *d != f.name {
                                // would be nice to be able to bail, but can't from retain_mut
                                tracing::error!("found two different consts between enum variants");
                                return false;
                            }
                        } else {
                            discriminator = Some(f.name.clone());
                        }

                        name = Some(value.clone());
                        false
                    }
                    _ => true,
                });

                let name = name.context("failed to find discriminator value")?;
                Ok(Variant { name, fields })
            })
            .collect::<anyhow::Result<Vec<_>>>()?;

        let discriminator = discriminator.context("failed to detect enum discriminator")?;
        Ok((discriminator, variants))
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
    /// Discriminator value that identifies this variant.
    name: String,
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

    /// A string constant, used as an enum discriminator value.
    StringConst(String),
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

    fn from_schema_object(obj: SchemaObject) -> anyhow::Result<Self> {
        let result = match &obj.instance_type {
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
                InstanceType::String => {
                    // String consts are the only const / enum values we support, for now.
                    // Early return so we don't hit the checks for these two below.
                    if let Some(value) = obj.const_value {
                        let serde_json::Value::String(s) = value else {
                            bail!("unsupported: non-string constant as field type");
                        };
                        return Ok(Self::StringConst(s));
                    }
                    if let Some(values) = obj.enum_values {
                        let Ok([value]): Result<[_; 1], _> = values.try_into() else {
                            bail!("unsupported: enum as field type");
                        };
                        let serde_json::Value::String(s) = value else {
                            bail!("unsupported: non-string constant as field type");
                        };
                        return Ok(Self::StringConst(s));
                    }

                    match obj.format.as_deref() {
                        None => Self::String,
                        Some("date-time") => Self::DateTime,
                        Some("uri") => Self::Uri,
                        Some(f) => bail!("unsupported string format: `{f:?}`"),
                    }
                }
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
        };

        // If we didn't hit the early return above, check that there's no const or enum value(s).
        ensure!(obj.const_value.is_none(), "unsupported const_value");
        ensure!(obj.enum_values.is_none(), "unsupported enum_values");

        Ok(result)
    }

    fn to_csharp_typename(&self) -> Cow<'_, str> {
        match self {
            Self::Bool => "bool".into(),
            Self::Int16 => "short".into(),
            Self::Int32 => "int".into(),
            Self::Int64 => "long".into(),
            Self::UInt16 => "ushort".into(),
            Self::UInt64 => "ulong".into(),
            Self::String => "string".into(),
            Self::DateTime => "DateTime".into(),
            Self::Uri => "string".into(),
            Self::JsonObject => "Object".into(),
            Self::Map { value_ty } => {
                format!("Dictionary<string, {}>", value_ty.to_csharp_typename()).into()
            }
            Self::List(field_type) | Self::Set(field_type) => {
                format!("List<{}>", field_type.to_csharp_typename()).into()
            }
            Self::SchemaRef(name) => name.clone().into(),
            Self::StringConst(_) => {
                unreachable!("FieldType::const should never be exposed to template code")
            }
        }
    }

    fn to_go_typename(&self) -> Cow<'_, str> {
        match self {
            Self::Bool => "bool".into(),
            Self::Int16 => "int16".into(),
            Self::Int32 => "int32".into(),
            Self::Int64 => "int64".into(),
            Self::UInt16 => "uint16".into(),
            Self::UInt64 => "uint64".into(),
            Self::Uri | Self::String => "string".into(),
            Self::DateTime => "time.Time".into(),
            Self::JsonObject => "map[string]any".into(),
            Self::Map { value_ty } => format!("map[string]{}", value_ty.to_go_typename()).into(),
            Self::List(field_type) | Self::Set(field_type) => {
                format!("[]{}", field_type.to_go_typename()).into()
            }
            Self::SchemaRef(name) => name.clone().into(),
            Self::StringConst(_) => {
                unreachable!("FieldType::const should never be exposed to template code")
            }
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
            Self::StringConst(_) => unreachable!("FieldType::const should never be exposed to template code"),
        }
    }

    fn to_js_typename(&self) -> Cow<'_, str> {
        match self {
            Self::Bool => "boolean".into(),
            Self::Int16 | Self::UInt16 | Self::Int32 | Self::Int64 | Self::UInt64 => {
                "number".into()
            }
            Self::String | Self::Uri => "string".into(),
            Self::DateTime => "Date | null".into(),
            Self::JsonObject => "any".into(),
            Self::List(field_type) | Self::Set(field_type) => {
                format!("{}[]", field_type.to_js_typename()).into()
            }
            Self::Map { value_ty } => {
                format!("{{ [key: string]: {} }}", value_ty.to_js_typename()).into()
            }
            Self::SchemaRef(name) => name.clone().into(),
            Self::StringConst(_) => {
                unreachable!("FieldType::const should never be exposed to template code")
            }
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
            Self::StringConst(_) => unreachable!("FieldType::const should never be exposed to template code"),
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
            Self::SchemaRef(name) => name.clone().into(),
            Self::Uri => "str".into(),
            Self::JsonObject => "t.Dict[str, t.Any]".into(),
            Self::Set(field_type) | Self::List(field_type) => {
                format!("t.List[{}]", field_type.to_python_typename()).into()
            }
            Self::Map { value_ty } => {
                format!("t.Dict[str, {}]", value_ty.to_python_typename()).into()
            }
            Self::StringConst(_) => {
                unreachable!("FieldType::const should never be exposed to template code")
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
                ensure_no_args(args, "is_schema_ref")?;
                Ok(matches!(**self, Self::SchemaRef(_)).into())
            }
            "is_set" => {
                ensure_no_args(args, "is_set")?;
                Ok(matches!(**self, Self::Set(_)).into())
            }
            "is_list" => {
                ensure_no_args(args, "is_list")?;
                Ok(matches!(**self, Self::List(_)).into())
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
