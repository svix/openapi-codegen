use fs_err as fs;

use crate::api::Api;

pub(crate) fn write_api_and_types(api: &Api) -> anyhow::Result<()> {
    let serialized = ron::ser::to_string_pretty(
        api,
        ron::ser::PrettyConfig::new().extensions(ron::extensions::Extensions::IMPLICIT_SOME),
    )?;
    fs::write("debug.ron", serialized)?;

    Ok(())
}
