use std::io::{BufRead, ErrorKind, Seek};
use std::{collections::BTreeSet, io, process::Command, sync::Mutex};

use anyhow::Context as _;
use camino::Utf8Path;
use serde::de::DeserializeOwned;

pub(crate) fn get_schema_name(maybe_ref: Option<String>) -> Option<String> {
    let r = maybe_ref?;
    let schema_name = r.strip_prefix("#/components/schemas/");
    if schema_name.is_none() {
        tracing::warn!(
            component_ref = r,
            "missing #/components/schemas/ prefix on component ref"
        );
    };
    Some(schema_name?.to_owned())
}

pub(crate) fn run_formatter(path: &Utf8Path) {
    let Some(file_ext) = path.extension() else {
        return;
    };

    let (formatter, args) = match file_ext {
        "rs" => ("rustfmt", ["+nightly", "--edition", "2021"].as_slice()),
        "go" => ("gofmt", ["-w"].as_slice()),
        "kt" => ("ktfmt", ["--kotlinlang-style"].as_slice()),
        _ => {
            tracing::debug!("no known formatter for {file_ext} files");
            return;
        }
    };

    let result = Command::new(formatter).args(args).arg(path).status();
    match result {
        Ok(exit_status) if exit_status.success() => {}
        Ok(exit_status) => {
            tracing::warn!(exit_status = exit_status.code(), "`{formatter}` failed");
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            // only print one error per formatter that's not found
            static NOT_FOUND_LOGGED_FOR: Mutex<BTreeSet<&str>> = Mutex::new(BTreeSet::new());
            if NOT_FOUND_LOGGED_FOR.lock().unwrap().insert(formatter) {
                tracing::warn!("`{formatter}` not found");
            }
        }
        Err(e) => {
            tracing::warn!(
                error = &e as &dyn std::error::Error,
                "running `{formatter}` failed"
            );
        }
    }
}

// Copied from https://github.com/jplatte/hinoki/blob/cc79b743d450c410a5e5e8abb1a7de16ec73797b/components/core/src/frontmatter.rs

/// Looks for TOML frontmatter in the given reader and parses it if found.
///
/// If the input does not start with a frontmatter delimiter (line of `+++` with
/// optional trailing whitespace), returns `Ok(T::default())`. If the frontmatter
/// delimiter is found, parses all the lines between that one and the next one
/// found. If successful, the input will be advanced such that the remaining
/// content after the frontmatter can be processed from it.
pub(crate) fn parse_frontmatter<T>(input: impl BufRead + Seek) -> anyhow::Result<T>
where
    T: Default + DeserializeOwned,
{
    // Read at most 256 bytes at once. Avoids loading lots of irrelevant data
    // into memory for binary files.
    let mut limited = input.take(256);

    macro_rules! bail_default {
        () => {{
            let mut input = limited.into_inner();
            input.rewind()?;
            return Ok(T::default());
        }};
    }

    let mut buf = String::new();
    if let Err(e) = limited.read_line(&mut buf) {
        match e.kind() {
            // Invalid UTF-8
            ErrorKind::InvalidData => bail_default!(),
            _ => return Err(e.into()),
        }
    }

    if buf.trim_end() != "+++" {
        bail_default!();
    }

    // If frontmatter delimiter was found, don't limit reading anymore.
    let mut input = limited.into_inner();
    buf.clear();
    loop {
        input.read_line(&mut buf)?;
        if buf
            .lines()
            .next_back()
            .is_some_and(|l| l.trim_end() == "+++")
        {
            let frontmatter_end_idx = buf.rfind("+++").expect("already found once");
            buf.truncate(frontmatter_end_idx);
            break;
        }
    }

    toml::from_str(&buf).context("parsing frontmatter")
}
