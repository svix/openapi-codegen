use std::{collections::BTreeSet, io, process::Command, sync::Mutex};

use camino::Utf8PathBuf;

pub(crate) fn execute_command(command: &'static str, args: &[&str], paths: &Vec<Utf8PathBuf>) {
    let result = Command::new(command).args(args).args(paths).status();
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
