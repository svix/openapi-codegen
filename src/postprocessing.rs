use std::{collections::BTreeSet, io, process::Command, sync::Mutex};

use camino::Utf8Path;

#[derive(Debug, Clone, Copy)]
pub(crate) enum Postprocessor {
    Python,
    Rust,
    Go,
    Kotlin,
    CSharp,
}

impl Postprocessor {
    pub(crate) fn from_ext(ext: &str) -> Option<Self> {
        match ext {
            "py" => Some(Self::Python),
            "rs" => Some(Self::Rust),
            "go" => Some(Self::Go),
            "kt" => Some(Self::Kotlin),
            "cs" => Some(Self::CSharp),
            _ => {
                tracing::warn!("no known postprocessing command(s) for {ext} files");
                None
            }
        }
    }

    pub(crate) fn postprocess_path(&self, path: &Utf8Path) {
        for (command, args) in self.postprocessing_commands() {
            execute_postprocessing_command(path, command, args);
        }
    }

    pub(crate) fn should_postprocess_single_file(&self) -> bool {
        match self {
            Self::Rust | Self::Kotlin | Self::Go => true,
            Self::CSharp | Self::Python => false,
        }
    }

    fn postprocessing_commands(&self) -> &[(&'static str, &[&str])] {
        match self {
            Self::Python => &[
                ("ruff", &["check", "--no-respect-gitignore", "--fix"]), // First lint and remove unused imports
                (
                    "ruff", // Then sort imports
                    &["check", "--no-respect-gitignore", "--select", "I", "--fix"],
                ),
                ("ruff", &["format", "--no-respect-gitignore"]), // Then format the file
            ],
            Self::Rust => &[(
                "rustfmt",
                &[
                    "+nightly",
                    "--unstable-features",
                    "--skip-children",
                    "--edition",
                    "2021",
                ],
            )],
            Self::Go => &[("gofmt", &["-w"])],
            Self::Kotlin => &[("ktfmt", &["--kotlinlang-style"])],
            Self::CSharp => &[("dotnet", &["csharpier", "--fast", "--no-msbuild-check"])],
        }
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
