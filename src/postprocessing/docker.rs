use bollard::{
    container::{Config, CreateContainerOptions},
    secret::HostConfig,
    Docker,
};
use camino::Utf8PathBuf;
use rand::Rng;

static IMAGE_NAME: &str = "svix/openapi-codegen-postprocess";

pub(crate) async fn execute_command(
    command: &'static str,
    args: &[&str],
    paths: &Vec<Utf8PathBuf>,
    output_dir: &Utf8PathBuf,
) -> anyhow::Result<()> {
    let s: String = rand::rng()
        .sample_iter(rand::distr::Alphanumeric)
        .take(15)
        .map(char::from)
        .collect();
    let docker = connect()?;
    let container_name = format!("openapi-codegen-postprocess-{s}");
    let mut entrypoint = vec![command.to_string()];
    for arg in args {
        entrypoint.push(arg.to_string());
    }
    for p in paths {
        let new_path = p
            .canonicalize_utf8()?
            .as_str()
            .replace(output_dir.canonicalize_utf8()?.as_str(), "/tmp");
        entrypoint.push(new_path);
    }
    let config = Config::<String> {
        image: Some(IMAGE_NAME.to_string()),
        host_config: Some(HostConfig {
            binds: Some(vec![format!(
                "{}:/tmp",
                output_dir.canonicalize_utf8()?.as_str()
            )]),
            ..Default::default()
        }),
        working_dir: Some("/tmp".to_string()),
        entrypoint: Some(entrypoint),
        ..Default::default()
    };
    let c = docker
        .create_container(
            Some(CreateContainerOptions::<String> {
                name: container_name,
                ..Default::default()
            }),
            config,
        )
        .await
        .unwrap();
    docker.start_container::<String>(&c.id, None).await.unwrap();

    Ok(())
}

fn connect() -> anyhow::Result<Docker> {
    Ok(Docker::connect_with_local_defaults()?)
}
