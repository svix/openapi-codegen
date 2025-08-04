use std::io::{BufWriter, Write as _};

use fs_err::File;

use crate::{api::Api, types::Types};

pub(crate) fn write_debug_files(api: &Api, types: &Types) -> anyhow::Result<()> {
    let mut api_file = BufWriter::new(File::create("api.ron")?);
    writeln!(api_file, "{api:#?}")?;

    let mut types_file = BufWriter::new(File::create("types.ron")?);
    writeln!(types_file, "{types:#?}")?;

    Ok(())
}
