use anyhow::{Context as _, bail, ensure};

use crate::{
    JsonValue,
    api::{
        get_schema_name,
        types::{EnumVariantType, Field, SimpleVariant, StructEnumRepr, TypeData},
    },
    utils::get_properties,
};

/// A wrapper around a Option<String>
///
/// Only allows value to be updated once. once updated, ant subsequent values must be the same
struct SameString(Option<String>);

impl SameString {
    fn update(&mut self, val: String) -> anyhow::Result<()> {
        match self.0.as_ref() {
            Some(current_val) => ensure!(*current_val == val),
            None => self.0 = Some(val),
        }
        Ok(())
    }
    fn inner(self) -> Option<String> {
        self.0
    }
}

impl TypeData {
    pub(super) fn inline_struct_enum(one_of: &JsonValue, fields: &[Field]) -> anyhow::Result<Self> {
        let one_of = one_of.as_array().context("oneOf must be an array")?;

        let mut discriminator_field = SameString(None);
        let mut content_field = SameString(None);
        let mut variants = vec![];

        let mut process_one_of = |variant: &JsonValue| {
            let (variant_discriminator_name, discriminator) =
                get_discriminator(variant).context("get struct-enum discriminator")?;
            discriminator_field.update(variant_discriminator_name)?;

            let properties = get_properties(variant).context("get struct-enum properties")?;
            let len = properties.len();
            ensure!(
                (1..=2).contains(&len),
                "Found struct enum variant with {len} properties, expected 1 or 2"
            );
            if properties.len() == 1 {
                variants.push(SimpleVariant {
                    name: discriminator,
                    content: EnumVariantType::Ref {
                        schema_ref: None,
                        inner: None,
                    },
                });
            } else {
                let (variant_content_field, content) =
                    get_content(variant).context("get struct-enum content")?;
                content_field.update(variant_content_field)?;

                variants.push(SimpleVariant {
                    name: discriminator,
                    content,
                });
            }

            Ok(())
        };

        for (idx, s) in one_of.iter().enumerate() {
            process_one_of(s).with_context(|| format!("oneOf[{idx}]"))?;
        }

        Ok(Self::StructEnum {
            discriminator_field: discriminator_field
                .inner()
                .context("failed to find discriminator field")?,
            fields: fields.to_vec(),
            repr: StructEnumRepr::AdjacentlyTagged {
                content_field: content_field
                    .inner()
                    .context("failed to find content field")?,
                variants,
            },
        })
    }
}

fn get_content(variant: &JsonValue) -> anyhow::Result<(String, EnumVariantType)> {
    for (prop_name, prop_schema) in get_properties(variant)? {
        if prop_schema["type"] == "object" {
            let ty = TypeData::from_object_schema(prop_schema)?;
            let TypeData::Struct { fields } = ty else {
                bail!("Expected obj to be a struct");
            };

            return Ok((prop_name.to_owned(), EnumVariantType::Struct { fields }));
        } else if let Some(reference) = prop_schema["$ref"].as_str() {
            return Ok((
                prop_name.to_owned(),
                EnumVariantType::Ref {
                    schema_ref: Some(get_schema_name(reference).unwrap()),
                    inner: None,
                },
            ));
        }
    }

    bail!("Failed to find content on struct enum")
}

fn get_discriminator(obj: &JsonValue) -> anyhow::Result<(String, String)> {
    let mut discriminator_field_name = None;
    let mut discriminator = None;

    for (prop_name, prop_schema) in get_properties(obj)? {
        if let Some(enum_value) = &prop_schema.get("enum")
            && let Some(enum_list) = enum_value.as_array()
            && let [value] = enum_list.as_slice()
        {
            let value = value
                .as_str()
                .context("Expected discriminator field name to be a string")?;

            discriminator_field_name = Some(prop_name.clone());
            discriminator = Some(value.to_owned());
        }
    }

    let Some(discriminator_field_name) = discriminator_field_name else {
        bail!("Unable to figure out discriminator field name")
    };
    let Some(discriminator) = discriminator else {
        bail!("Unable to figure out discriminator")
    };

    Ok((discriminator_field_name, discriminator))
}
