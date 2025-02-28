mod cli;
mod docker;

use camino::{Utf8Path, Utf8PathBuf};
use std::cell::RefCell;

use crate::PostprocessorOptions;

#[derive(Debug, Clone)]
pub(crate) enum CommandRunner {
    Cli,
    Docker,
}

#[derive(Debug, Clone)]
pub(crate) struct Postprocessor {
    files_to_process: RefCell<Vec<Utf8PathBuf>>,
    postprocessor_lang: PostprocessorLanguage,
    output_dir: Utf8PathBuf,
    runner: CommandRunner,
}
impl Postprocessor {
    fn new(
        postprocessor_lang: PostprocessorLanguage,
        output_dir: Utf8PathBuf,
        postprocessor_options: &PostprocessorOptions,
    ) -> Self {
        let runner = {
            if postprocessor_options.use_docker_backend {
                CommandRunner::Docker
            } else {
                CommandRunner::Cli
            }
        };
        Self {
            files_to_process: RefCell::new(Vec::new()),
            postprocessor_lang,
            output_dir,
            runner,
        }
    }
    pub(crate) fn from_ext(
        ext: &str,
        output_dir: &Utf8Path,
        postprocessor_options: &PostprocessorOptions,
    ) -> Self {
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
        Self::new(lang, output_dir.to_path_buf(), postprocessor_options)
    }

    pub(crate) async fn run_postprocessor(&self) -> anyhow::Result<()> {
        match self.postprocessor_lang {
            // pass each file to postprocessor at once
            PostprocessorLanguage::Java | PostprocessorLanguage::Rust => {
                let commands = self.postprocessor_lang.postprocessing_commands();
                for (command, args) in commands {
                    let paths = { self.files_to_process.borrow().clone() };
                    self.execute_command(command, args, &paths).await?;
                }
            }
            // pass output dir to postprocessor
            PostprocessorLanguage::Ruby
            | PostprocessorLanguage::Python
            | PostprocessorLanguage::Go
            | PostprocessorLanguage::Kotlin
            | PostprocessorLanguage::CSharp
            | PostprocessorLanguage::TypeScript => {
                let commands = self.postprocessor_lang.postprocessing_commands();
                for (command, args) in commands {
                    self.execute_command(command, args, &vec![self.output_dir.clone()])
                        .await?;
                }
            }
            PostprocessorLanguage::Unknown => (),
        }
        Ok(())
    }

    async fn execute_command(
        &self,
        command: &'static str,
        args: &[&str],
        paths: &Vec<Utf8PathBuf>,
    ) -> anyhow::Result<()> {
        match self.runner {
            CommandRunner::Cli => cli::execute_command(command, args, paths),
            CommandRunner::Docker => {
                docker::execute_command(command, args, paths, &self.output_dir).await?
            }
        }
        Ok(())
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
