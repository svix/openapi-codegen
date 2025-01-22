use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    io::{BufRead, ErrorKind, Seek},
};
use std::{collections::BTreeSet, io, process::Command, sync::Mutex};

use anyhow::Context as _;
use camino::Utf8Path;
use serde::{
    de::DeserializeOwned,
    ser::{Serialize, SerializeSeq as _, Serializer},
};

pub(crate) fn get_schema_name(maybe_ref: Option<&str>) -> Option<String> {
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

pub(crate) fn run_postprocessing(path: &Utf8Path) {
    let Some(file_ext) = path.extension() else {
        return;
    };

    let postprocessing_tasks: &[(&str, &[&str])] = {
        match file_ext {
            "py" => &[
                // fixme: the ordering of the commands is important, maybe ensure the order in a more robust way
                ("ruff", ["check", "--fix"].as_slice()), // First lint and remove unused imports
                ("ruff", ["check", "--select", "I", "--fix"].as_slice()), // Then sort imports
                ("ruff", ["format"].as_slice()),         // Then format the file
            ],
            "rs" => &[(
                "rustfmt",
                [
                    "+nightly",
                    "--unstable-features",
                    "--skip-children",
                    "--edition",
                    "2021",
                ]
                .as_slice(),
            )],
            "go" => &[("gofmt", ["-w"].as_slice())],
            "kt" => &[("ktfmt", ["--kotlinlang-style"].as_slice())],
            _ => {
                tracing::debug!("no known postprocessing command(s) for {file_ext} files");
                return;
            }
        }
    };
    for (command, args) in postprocessing_tasks {
        execute_postprocessing_command(path, command, args);
    }
}

fn execute_postprocessing_command(path: &Utf8Path, command: &'static str, args: &[&str]) {
    let result = Command::new(command).args(args).arg(path).status();
    match result {
        Ok(exit_status) if exit_status.success() => {}
        Ok(exit_status) => {
            tracing::warn!(exit_status = exit_status.code(), "`{command}` failed");
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            // only print one error per command that's not found
            static NOT_FOUND_LOGGED_FOR: Mutex<BTreeSet<&str>> = Mutex::new(BTreeSet::new());
            if NOT_FOUND_LOGGED_FOR.lock().unwrap().insert(command) {
                tracing::warn!("`{command}` not found");
            }
        }
        Err(e) => {
            tracing::warn!(
                error = &e as &dyn std::error::Error,
                "running `{command}` failed"
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

pub(crate) fn serialize_btree_map_values<K, V, S>(
    map: &BTreeMap<K, V>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    V: Serialize,
    S: Serializer,
{
    let mut seq = serializer.serialize_seq(Some(map.len()))?;
    for item in map.values() {
        seq.serialize_element(item)?;
    }
    seq.end()
}

pub(crate) fn sha256sum_string(s: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(s.as_bytes());
    let hash = hasher.finalize();
    format!("{hash:x}")
}
