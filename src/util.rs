use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::{collections::BTreeSet, io, process::Command, sync::Mutex};

use camino::Utf8Path;
use serde::ser::{Serialize, SerializeSeq as _, Serializer};

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
