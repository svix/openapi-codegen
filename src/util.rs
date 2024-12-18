use std::{collections::BTreeSet, io, process::Command, sync::Mutex};

use camino::Utf8Path;

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
        "go" => ("gofmt", [].as_slice()),
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
