use bollard::{
    Docker,
    query_parameters::{
        CreateContainerOptionsBuilder, CreateImageOptionsBuilder, LogsOptionsBuilder,
        RemoveContainerOptionsBuilder, StartContainerOptionsBuilder, WaitContainerOptionsBuilder,
    },
    secret::{ContainerCreateBody, HostConfig, Mount, MountTypeEnum},
};
use camino::Utf8PathBuf;
use futures::{StreamExt, TryStreamExt};
use std::io::Write;
use tempfile::NamedTempFile;
use tokio::sync::Semaphore;

use crate::CodegenLanguage;
static IMAGE_PULL_LOCK: Semaphore = Semaphore::const_new(1);

#[derive(Clone)]
pub struct ContainerizedPostprocessor<'a> {
    files_to_process: &'a [Utf8PathBuf],
    postprocessor_lang: CodegenLanguage,
    output_dir: Utf8PathBuf,
    docker: Docker,
}

impl<'a> ContainerizedPostprocessor<'a> {
    pub fn new(
        postprocessor_lang: CodegenLanguage,
        output_dir: Utf8PathBuf,
        files_to_process: &'a [Utf8PathBuf],
    ) -> Self {
        let docker = Docker::connect_with_local_defaults().unwrap();
        Self {
            files_to_process,
            postprocessor_lang,
            output_dir,
            docker,
        }
    }

    pub async fn run_postprocessor(&self) -> anyhow::Result<()> {
        match self.postprocessor_lang {
            // pass each file to postprocessor at once
            CodegenLanguage::Java | CodegenLanguage::Rust => {
                let commands = self.postprocessor_lang.sample_postprocessing_commands();
                for (command, args) in commands {
                    self.run_command_in_docker(command, args, self.files_to_process)
                        .await?;
                }
            }
            // pass output dir to postprocessor
            CodegenLanguage::Ruby
            | CodegenLanguage::Php
            | CodegenLanguage::Python
            | CodegenLanguage::Go
            | CodegenLanguage::Kotlin
            | CodegenLanguage::CSharp
            | CodegenLanguage::TypeScript => {
                let commands = self.postprocessor_lang.sample_postprocessing_commands();
                for (command, args) in commands {
                    self.run_command_in_docker(
                        command,
                        args,
                        std::slice::from_ref(&self.output_dir),
                    )
                    .await?;
                }
            }
            CodegenLanguage::Unknown | CodegenLanguage::Shell => (),
        }
        Ok(())
    }
    async fn pull_image_if_not_exists(&self, image: &str) -> anyhow::Result<()> {
        match self.docker.inspect_image(image).await {
            Ok(_) => (),
            Err(err) => match err {
                bollard::errors::Error::DockerResponseServerError {
                    status_code: 404, ..
                } => {
                    let guard = IMAGE_PULL_LOCK.acquire().await.unwrap();
                    // after we get the log, check again
                    if let Err(err) = self.docker.inspect_image(image).await {
                        match err {
                            bollard::errors::Error::DockerResponseServerError {
                                status_code: 404,
                                ..
                            } => {}
                            _ => anyhow::bail!("{err:?}"),
                        }
                    } else {
                        return Ok(());
                    }

                    let options = Some(CreateImageOptionsBuilder::new().from_image(image).build());

                    println!("Pulling image: {image}");
                    let mut res = self.docker.create_image(options, None, None);
                    while let Some(res) = res.next().await {
                        let _res = res?;
                    }
                    println!("Done pulling image: {image}");

                    drop(guard);
                }
                _ => {
                    anyhow::bail!("{err:?}");
                }
            },
        }

        Ok(())
    }

    async fn run_command_in_docker(
        &self,
        command: &'static str,
        args: &[&str],
        paths: &[Utf8PathBuf],
    ) -> anyhow::Result<()> {
        self.pull_image_if_not_exists("ghcr.io/svix/openapi-codegen:20251003-333")
            .await?;
        let create_container_ops = CreateContainerOptionsBuilder::new().build();

        let mut cmd = vec![];
        cmd.push(command.to_string());
        for a in args {
            cmd.push(a.to_string());
        }
        for p in paths {
            cmd.push(p.to_string());
        }
        let script = format!("#!/bin/sh\n{}", cmd.join(" "));

        let mut entrypoint_file = NamedTempFile::new().unwrap();
        entrypoint_file.write_all(script.as_bytes()).unwrap();
        let entrypoint_file_path = entrypoint_file.path();

        let host_config = HostConfig {
            mounts: Some(vec![
                Mount {
                    typ: Some(MountTypeEnum::BIND),
                    target: Some(entrypoint_file_path.to_str().unwrap().to_string()),
                    source: Some(entrypoint_file_path.to_str().unwrap().to_string()),
                    ..Default::default()
                },
                Mount {
                    typ: Some(MountTypeEnum::BIND),
                    target: Some(self.output_dir.as_str().to_string()),
                    source: Some(self.output_dir.as_str().to_string()),
                    ..Default::default()
                },
            ]),
            ..Default::default()
        };

        let config = ContainerCreateBody {
            image: Some("ghcr.io/svix/openapi-codegen:20251003-333".to_string()),
            working_dir: Some(self.output_dir.as_str().to_string()),
            host_config: Some(host_config),
            entrypoint: Some(vec![
                "/bin/sh".to_string(),
                entrypoint_file_path.to_str().unwrap().to_string(),
            ]),
            ..Default::default()
        };
        let c = self
            .docker
            .create_container(Some(create_container_ops), config)
            .await?;

        self.docker
            .start_container(&c.id, Some(StartContainerOptionsBuilder::new().build()))
            .await?;

        let res = self
            .docker
            .wait_container(
                &c.id,
                Some(
                    WaitContainerOptionsBuilder::new()
                        .condition("not-running")
                        .build(),
                ),
            )
            .next()
            .await
            .unwrap();

        let exit_code = match res {
            Ok(r) => r.status_code,
            Err(_e) => -1,
        };

        if exit_code != 0 {
            let logs: Vec<_> = self
                .docker
                .logs(
                    &c.id,
                    Some(LogsOptionsBuilder::new().stderr(true).stdout(true).build()),
                )
                .try_collect()
                .await?;
            for l in logs {
                match l {
                    bollard::container::LogOutput::StdErr { message } => {
                        let mut stderr = std::io::stderr().lock();
                        stderr.write_all(&message)?;
                        stderr.flush()?;
                    }
                    bollard::container::LogOutput::StdOut { message } => {
                        let mut stdout = std::io::stdout().lock();
                        stdout.write_all(&message)?;
                        stdout.flush()?;
                    }
                    bollard::container::LogOutput::Console { message }
                    | bollard::container::LogOutput::StdIn { message } => {
                        let mut stdout = std::io::stdout().lock();
                        stdout.write_all(&message)?;
                        stdout.flush()?;
                    }
                }
            }

            anyhow::bail!("Container exited with code {exit_code}");
        } else {
            self.docker
                .remove_container(&c.id, Some(RemoveContainerOptionsBuilder::new().build()))
                .await?;
        }

        Ok(())
    }
}

impl CodegenLanguage {
    pub fn sample_postprocessing_commands(&self) -> &[(&'static str, &[&str])] {
        match self {
            Self::Unknown | Self::Shell => &[],
            // https://github.com/astral-sh/ruff
            Self::Python => &[
                ("ruff", &["format", "--no-respect-gitignore"]), // Then format the file
            ],
            Self::Rust => &[(
                "rustfmt",
                &[
                    "+nightly-2025-02-27",
                    "--unstable-features",
                    "--skip-children",
                    "--edition",
                    "2021",
                ],
            )],
            Self::Go => &[("golines", &["-w", "--base-formatter='gofumpt'"])],
            // https://github.com/facebook/ktfmt
            Self::Kotlin => &[("ktfmt", &["--kotlinlang-style"])],
            // https://github.com/belav/csharpier
            Self::CSharp => &[(
                "csharpier",
                &[
                    "format",
                    "--no-cache",
                    "--skip-validation",
                    "--no-msbuild-check",
                ],
            )],
            // https://github.com/google/google-java-format
            Self::Java => &[("google-java-format", &["-i", "-a"])],
            // https://github.com/biomejs/biome
            Self::TypeScript => &[(
                "biome",
                &[
                    "format",
                    "--trailing-commas=es5",
                    "--indent-style=space",
                    "--line-width=90",
                    "--write",
                ],
            )],
            // https://github.com/fables-tales/rubyfmt
            Self::Ruby => &[("rubyfmt", &["-i", "--include-gitignored", "--fail-fast"])],
            Self::Php => &[(
                // https://github.com/PHP-CS-Fixer/PHP-CS-Fixer
                "php84",
                &[
                    "/usr/share/php-cs-fixer.phar",
                    "--no-ansi",
                    "fix",
                    "--using-cache=no",
                    "--rules=no_unused_imports,@Symfony",
                ],
            )],
        }
    }
}
