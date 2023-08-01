use std::path::{Path, PathBuf};

use bollard::container::CreateContainerOptions;
use bollard::service::Mount;
use eyre::Result;
use tracing::{error, info, warn};

use crate::util::config::PeckishConfig;

pub async fn test_packages(config: PeckishConfig) -> Result<()> {
    for output in config.output.into_iter() {
        match output {
            crate::util::config::ConfiguredProducer::Tarball(producer) => {
                info!("testing producer: {}", producer.name);
                run_command_in_docker_with_mount(
                    "tar".into(),
                    producer.path.clone(),
                    file_name(&producer.path),
                    vec!["tar".into(), "tfv".into(), app_path(&producer.path)],
                    "alpine:latest".into(),
                )
                .await?;
            }
            crate::util::config::ConfiguredProducer::Arch(producer) => {
                info!("testing producer: {}", producer.name);
                run_command_in_docker_with_mount(
                    "arch".into(),
                    producer.path.clone(),
                    file_name(&producer.path),
                    vec![
                        "pacman".into(),
                        "--noconfirm".into(),
                        "-U".into(),
                        app_path(&producer.path),
                    ],
                    "archlinux:latest".into(),
                )
                .await?;
            }
            crate::util::config::ConfiguredProducer::Deb(producer) => {
                info!("testing producer: {}", producer.name);
                run_command_in_docker_with_mount(
                    "deb".into(),
                    producer.path.clone(),
                    file_name(&producer.path),
                    vec!["dpkg".into(), "-i".into(), app_path(&producer.path)],
                    "debian:latest".into(),
                )
                .await?;

                run_command_in_docker_with_mount(
                    "deb".into(),
                    producer.path.clone(),
                    file_name(&producer.path),
                    vec!["dpkg".into(), "-i".into(), app_path(&producer.path)],
                    "ubuntu:latest".into(),
                )
                .await?;
            }
            crate::util::config::ConfiguredProducer::Rpm(producer) => {
                info!("testing producer: {}", producer.name);
                run_command_in_docker_with_mount(
                    "rpm".into(),
                    producer.path.clone(),
                    file_name(&producer.path),
                    vec!["rpm".into(), "-i".into(), app_path(&producer.path)],
                    "fedora:latest".into(),
                )
                .await?;
            }
            _ => warn!("not handling producer: {}", output.name()),
        }
    }

    Ok(())
}

fn file_name(src: &Path) -> String {
    src.file_name().unwrap().to_string_lossy().to_string()
}

fn app_path(src: &Path) -> String {
    let file_name = file_name(src);
    format!("/app/{file_name}")
}

async fn run_command_in_docker_with_mount(
    kind: String,
    mount_src: PathBuf,
    mount_dest: String,
    command: Vec<String>,
    image: String,
) -> Result<()> {
    let name = format!("peckish-tester-{kind}");
    let docker = bollard::Docker::connect_with_local_defaults()?;

    // create the container from $image
    let container = docker
        .create_container(
            Some(CreateContainerOptions {
                name: name.clone(),
                platform: None,
            }),
            bollard::container::Config {
                image: Some(image),
                cmd: Some(command),
                host_config: Some(bollard::service::HostConfig {
                    mounts: Some(vec![Mount {
                        source: Some(mount_src.canonicalize()?.to_string_lossy().to_string()),
                        target: Some(format!("/app/{}", mount_dest)),
                        typ: Some(bollard::service::MountTypeEnum::BIND),
                        ..Default::default()
                    }]),
                    ..Default::default()
                }),
                ..Default::default()
            },
        )
        .await?;

    // start the container
    docker
        .start_container::<String>(&container.id, None)
        .await?;

    use futures_util::TryStreamExt;
    // wait for the container to finish
    let results = docker
        .wait_container::<String>(&container.id, None)
        .try_collect::<Vec<_>>()
        .await?;

    // assert container exited successfully
    // if it didn't, dump logs
    if results[0].status_code != 0 {
        let logs = docker
            .logs::<String>(
                &container.id,
                Some(bollard::container::LogsOptions::<String> {
                    stdout: true,
                    stderr: true,
                    ..Default::default()
                }),
            )
            .try_collect::<Vec<_>>()
            .await?;
        error!("logs: {:?}", logs);
    }

    docker.remove_container(&name, None).await?;

    Ok(())
}
