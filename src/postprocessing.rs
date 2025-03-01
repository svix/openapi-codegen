use std::{cell::RefCell, collections::BTreeSet, io, process::Command, sync::Mutex};

use camino::{Utf8Path, Utf8PathBuf};

#[derive(Debug, Clone)]
pub(crate) struct Postprocessor {
    files_to_process: RefCell<Vec<Utf8PathBuf>>,
    postprocessor_lang: PostprocessorLanguage,
    output_dir: Utf8PathBuf,
}
impl Postprocessor {
    fn new(postprocessor_lang: PostprocessorLanguage, output_dir: Utf8PathBuf) -> Self {
        Self {
            files_to_process: RefCell::new(Vec::new()),
            postprocessor_lang,
            output_dir,
        }
    }
    pub(crate) fn from_ext(ext: &str, output_dir: &Utf8Path) -> Self {
        let lang = match ext {
            "py" => PostprocessorLanguage::Python,
            "rs" => PostprocessorLanguage::Rust,
            "go" => PostprocessorLanguage::Go,
            "kt" => PostprocessorLanguage::Kotlin,
            "cs" => PostprocessorLanguage::CSharp,
            "java" => PostprocessorLanguage::Java,
            "ts" => PostprocessorLanguage::TypeScript,
            "rb" => PostprocessorLanguage::Ruby,
            _ => {
                tracing::warn!("no known postprocessing command(s) for {ext} files");
                PostprocessorLanguage::Unknown
            }
        };
        Self::new(lang, output_dir.to_path_buf())
    }

    pub(crate) fn run_postprocessor(&self) {
        match self.postprocessor_lang {
            // pass each file to postprocessor at once
            PostprocessorLanguage::Go
            | PostprocessorLanguage::Java
            | PostprocessorLanguage::Rust => {
                let commands = self.postprocessor_lang.postprocessing_commands();
                for (command, args) in commands {
                    execute_command(command, args, &self.files_to_process.borrow());
                }
            }
            // pass output dir to postprocessor
            PostprocessorLanguage::Ruby
            | PostprocessorLanguage::Python
            | PostprocessorLanguage::Kotlin
            | PostprocessorLanguage::CSharp
            | PostprocessorLanguage::TypeScript => {
                let commands = self.postprocessor_lang.postprocessing_commands();
                for (command, args) in commands {
                    execute_command(command, args, &vec![self.output_dir.clone()]);
                }
            }
            PostprocessorLanguage::Unknown => (),
        }
    }
    pub(crate) fn add_path(&self, path: &Utf8Path) {
        let mut v = self.files_to_process.borrow_mut();
        v.push(path.to_path_buf());
    }
}
#[derive(Debug, Clone, Copy)]
pub(crate) enum PostprocessorLanguage {
    Python,
    Rust,
    Go,
    Kotlin,
    CSharp,
    Java,
    TypeScript,
    Ruby,
    Unknown,
}

impl PostprocessorLanguage {
    fn postprocessing_commands(&self) -> &[(&'static str, &[&str])] {
        match self {
            Self::Unknown => &[],
            // https://github.com/astral-sh/ruff
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
            // https://pkg.go.dev/golang.org/x/tools/cmd/goimports
            Self::Go => &[("goimports", &["-w"]), ("gofmt", &["-w"])],
            // https://github.com/facebook/ktfmt
            Self::Kotlin => &[("ktfmt", &["--kotlinlang-style"])],
            // https://github.com/belav/csharpier
            Self::CSharp => &[("dotnet", &["csharpier", "--fast", "--no-msbuild-check"])],
            // https://github.com/google/google-java-format
            Self::Java => &[("google-java-format", &["-i", "-a"])],
            // https://github.com/biomejs/biome
            Self::TypeScript => &[
                (
                    "biome",
                    &["lint", "--only=correctness/noUnusedImports", "--write"],
                ),
                (
                    "biome",
                    &[
                        "check",
                        "--formatter-enabled=false",
                        "--linter-enabled=false",
                        "--organize-imports-enabled=true",
                        "--write",
                    ],
                ),
                (
                    "biome",
                    &[
                        "format",
                        "--trailing-commas=es5",
                        "--indent-style=space",
                        "--line-width=90",
                        "--write",
                    ],
                ),
            ],
            // https://github.com/fables-tales/rubyfmt
            Self::Ruby => &[("rubyfmt", &["-i", "--include-gitignored", "--fail-fast"])],
        }
    }
}

fn execute_command(command: &'static str, args: &[&str], paths: &Vec<Utf8PathBuf>) {
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
