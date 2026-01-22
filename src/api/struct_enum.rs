use anyhow::{Context as _, bail, ensure};
use schemars::schema::{ObjectValidation, Schema, SchemaObject};

use crate::api::{
    get_schema_name,
    types::{EnumVariantType, Field, SimpleVariant, StructEnumRepr, TypeData},
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
    pub(super) fn inline_struct_enum(one_of: &[Schema], fields: &[Field]) -> anyhow::Result<Self> {
        let mut discriminator_field = SameString(None);
        let mut content_field = SameString(None);
        let mut variants = vec![];

        let mut process_one_of = |s: &Schema| {
            let variant = get_obj_validation(s)?;

            let (variant_discriminator_name, discriminator) = get_discriminator(variant)?;
            discriminator_field.update(variant_discriminator_name)?;

            let len = variant.properties.len();
            ensure!(
                (1..=2).contains(&len),
                "Found struct enum variant with {len} properties, expected 1 or 2"
            );
            if variant.properties.len() == 1 {
                variants.push(SimpleVariant {
                    name: discriminator,
                    content: EnumVariantType::Ref {
                        schema_ref: None,
                        inner: None,
                    },
                });
            } else {
                let (variant_content_field, content) = get_content(variant)?;
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

fn get_content(variant: &ObjectValidation) -> anyhow::Result<(String, EnumVariantType)> {
    for (p_name, p) in &variant.properties {
        let schema_obj = get_schema_obj(p)?;
        if let Some(obj) = &schema_obj.object {
            let ty = TypeData::from_object_schema(*obj.clone(), None)?;
            let TypeData::Struct { fields } = ty else {
                bail!("Expected obj to be a struct");
            };

            return Ok((p_name.to_owned(), EnumVariantType::Struct { fields }));
        }

        if let Some(schema_ref) = &schema_obj.reference {
            return Ok((
                p_name.to_owned(),
                EnumVariantType::Ref {
                    schema_ref: Some(get_schema_name(Some(schema_ref.as_str())).unwrap()),
                    inner: None,
                },
            ));
        }
    }

    bail!("Failed to find content on struct enum")
}

fn get_discriminator(obj: &ObjectValidation) -> anyhow::Result<(String, String)> {
    let mut discriminator_field_name = None;
    let mut discriminator = None;

    for (p_name, p) in &obj.properties {
        let schema_obj = get_schema_obj(p).with_context(|| p_name.to_owned())?;
        if let Some(enum_vals) = &schema_obj.enum_values
            && enum_vals.len() == 1
        {
            match &enum_vals[0].as_str() {
                Some(v) => {
                    discriminator_field_name = Some(p_name.clone());
                    discriminator = Some((*v).to_owned());
                }
                None => bail!("Expected discriminator field name to be a string"),
            }
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

fn get_schema_obj(s: &Schema) -> anyhow::Result<&SchemaObject> {
    match s {
        Schema::Bool(_) => bail!("unsupported bool schema"),
        Schema::Object(o) => Ok(o),
    }
}

fn get_obj_validation(s: &Schema) -> anyhow::Result<&ObjectValidation> {
    let Some(obj) = get_schema_obj(s)?.object.as_ref() else {
        bail!("unsupported: object type without further validation");
    };
    Ok(obj)
}
