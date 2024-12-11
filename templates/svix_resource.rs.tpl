#![allow(unused_imports)]

use super::PostOptions;
use crate::{
    apis::{{ resource.name | to_snake_case }}_api,
    error::Result,
    models::*,
    Configuration,
};

{% set resource_type_name = resource.name | to_upper_camel_case -%}

struct {{ resource_type_name }}<'a> {

impl<'a> {{ resource_type_name }}<'a> {
}
