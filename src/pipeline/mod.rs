use eyre::Result;
use log::*;

use crate::artifact::{Artifact, ArtifactProducer, SelfValidation};
use crate::util::config::{ConfiguredArtifact, ConfiguredProducer, PeckishConfig};

/// A pipeline that can run a given config. This is the main entrypoint for
/// running a peckish config.
pub struct Pipeline {}

impl Pipeline {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {}
    }

    pub async fn run(&self, config: PeckishConfig) -> Result<Vec<Box<dyn Artifact>>> {
        info!("running pipeline with {} steps!", config.output.len());
        let mut input_artifact: Box<dyn Artifact> = match config.input {
            ConfiguredArtifact::File(file) => Box::new(file),
            ConfiguredArtifact::Tarball(tarball) => Box::new(tarball),
            ConfiguredArtifact::Docker(docker) => Box::new(docker),
            ConfiguredArtifact::Arch(arch) => Box::new(arch),
            ConfiguredArtifact::Deb(deb) => Box::new(deb),
            ConfiguredArtifact::Rpm(rpm) => Box::new(rpm),
        };
        info!("input: {}", input_artifact.name());

        input_artifact.validate().await?;

        let mut output_artifacts: Vec<Box<dyn Artifact>> = vec![];

        for (i, producer) in config.output.iter().enumerate() {
            info!("step {}: {}", i + 1, producer.name());
            let next_artifact: Box<dyn Artifact> = match producer {
                ConfiguredProducer::File(producer) => {
                    producer.validate().await?;
                    Box::new(producer.produce_from(input_artifact.as_ref()).await?)
                }

                ConfiguredProducer::Tarball(producer) => {
                    producer.validate().await?;
                    Box::new(producer.produce_from(input_artifact.as_ref()).await?)
                }

                ConfiguredProducer::Docker(producer) => {
                    producer.validate().await?;
                    Box::new(producer.produce_from(input_artifact.as_ref()).await?)
                }

                ConfiguredProducer::Arch(producer) => {
                    producer.validate().await?;
                    Box::new(producer.produce_from(input_artifact.as_ref()).await?)
                }

                ConfiguredProducer::Deb(producer) => {
                    producer.validate().await?;
                    Box::new(producer.produce_from(input_artifact.as_ref()).await?)
                }

                ConfiguredProducer::Rpm(producer) => {
                    producer.validate().await?;
                    Box::new(producer.produce_from(input_artifact.as_ref()).await?)
                }
            };

            next_artifact.validate().await?;

            if config.pipeline {
                input_artifact = next_artifact.try_clone()?;
            }

            info!("created artifact: {}", next_artifact.name());
            output_artifacts.push(next_artifact);
        }

        let mut out = vec![];
        for artifact in output_artifacts.iter() {
            let artifact: Box<dyn Artifact> = artifact.try_clone()?;
            out.push(artifact);
        }

        Ok(output_artifacts)
    }
}

#[cfg(test)]
mod tests {
    use eyre::Result;
    use tokio::fs::File;
    use tokio::io::AsyncReadExt;

    use crate::artifact::file::{FileArtifact, FileProducer};
    use crate::artifact::tarball::TarballProducer;
    use crate::fs::TempDir;
    use crate::util::compression;
    use crate::util::config::Injection;

    use super::*;

    #[ctor::ctor]
    fn init() {
        crate::util::test_init();
    }

    #[tokio::test]
    async fn test_basic_pipeline_works() -> Result<()> {
        let tmp = TempDir::new().await?;
        let tar = tmp.path_view().join("cargo.toml.tar");

        let config = PeckishConfig {
            pipeline: true,
            input: ConfiguredArtifact::File(FileArtifact {
                name: "cargo dot toml".into(),
                paths: vec!["Cargo.toml".into()],
                strip_path_prefixes: None,
            }),
            output: vec![ConfiguredProducer::Tarball(TarballProducer {
                name: "cargo dot toml output".into(),
                path: tar.clone(),
                compression: compression::CompressionType::None,
                injections: vec![],
            })],
        };

        let pipeline = Pipeline::new();
        assert!(pipeline.run(config).await.is_ok());

        Ok(())
    }

    #[tokio::test]
    async fn test_move_injection_works() -> Result<()> {
        let tar_dir = TempDir::new().await?;
        let tar = tar_dir.path_view().join("cargo.toml.moveinject.tar");
        let tmp = TempDir::new().await?;

        let config = PeckishConfig {
            pipeline: true,
            input: ConfiguredArtifact::File(FileArtifact {
                name: "cargo dot toml".into(),
                paths: vec!["Cargo.toml".into()],
                strip_path_prefixes: None,
            }),
            output: vec![
                ConfiguredProducer::Tarball(TarballProducer {
                    name: "cargo dot toml output".into(),
                    path: tar.clone(),
                    compression: compression::CompressionType::None,
                    injections: vec![Injection::Move {
                        src: "Cargo.toml".into(),
                        dest: "Cargo-2.toml".into(),
                    }],
                }),
                ConfiguredProducer::File(FileProducer {
                    name: "unwrapper".into(),
                    path: tmp.path_view(),
                    preserve_empty_directories: None,
                    injections: vec![],
                }),
            ],
        };

        let pipeline = Pipeline::new();
        pipeline.run(config).await?;
        assert!(tmp.path_view().join("Cargo-2.toml").exists());
        assert!(!tmp.path_view().join("Cargo.toml").exists());

        Ok(())
    }

    #[tokio::test]
    async fn test_copy_injection_works() -> Result<()> {
        let tar_dir = TempDir::new().await?;
        let tar = tar_dir.path_view().join("cargo.toml.copyinject.tar");
        let tmp = TempDir::new().await?;

        let config = PeckishConfig {
            pipeline: true,
            input: ConfiguredArtifact::File(FileArtifact {
                name: "cargo dot toml".into(),
                paths: vec!["Cargo.toml".into()],
                strip_path_prefixes: None,
            }),
            output: vec![
                ConfiguredProducer::Tarball(TarballProducer {
                    name: "cargo dot toml output".into(),
                    path: tar.clone(),
                    compression: compression::CompressionType::None,
                    injections: vec![Injection::Copy {
                        src: "Cargo.toml".into(),
                        dest: "Cargo-2.toml".into(),
                    }],
                }),
                ConfiguredProducer::File(FileProducer {
                    name: "unwrapper".into(),
                    path: tmp.path_view(),
                    preserve_empty_directories: None,
                    injections: vec![],
                }),
            ],
        };

        let pipeline = Pipeline::new();
        pipeline.run(config).await?;
        assert!(tmp.path_view().join("Cargo-2.toml").exists());
        assert!(tmp.path_view().join("Cargo.toml").exists());

        Ok(())
    }

    #[tokio::test]
    async fn test_symlink_injection_works() -> Result<()> {
        let tar_dir = TempDir::new().await?;
        let tar = tar_dir.path_view().join("cargo.toml.symlinkinject.tar");
        let tmp = TempDir::new().await?;

        let config = PeckishConfig {
            pipeline: true,
            input: ConfiguredArtifact::File(FileArtifact {
                name: "cargo dot toml".into(),
                paths: vec!["Cargo.toml".into()],
                strip_path_prefixes: None,
            }),
            output: vec![
                ConfiguredProducer::Tarball(TarballProducer {
                    name: "cargo dot toml output".into(),
                    path: tar.clone(),
                    compression: compression::CompressionType::None,
                    injections: vec![Injection::Symlink {
                        src: "Cargo.toml".into(),
                        dest: "Cargo-2.toml".into(),
                    }],
                }),
                ConfiguredProducer::File(FileProducer {
                    name: "unwrapper".into(),
                    path: tmp.path_view(),
                    preserve_empty_directories: None,
                    injections: vec![],
                }),
            ],
        };

        let pipeline = Pipeline::new();
        pipeline.run(config).await?;
        assert!(tmp.path_view().join("Cargo-2.toml").is_symlink());
        assert!(tmp.path_view().join("Cargo.toml").exists());

        Ok(())
    }

    #[tokio::test]
    async fn test_touch_injection_works() -> Result<()> {
        let tar_dir = TempDir::new().await?;
        let tar = tar_dir.path_view().join("cargo.toml.touchinject.tar");
        let tmp = TempDir::new().await?;

        let config = PeckishConfig {
            pipeline: true,
            input: ConfiguredArtifact::File(FileArtifact {
                name: "cargo dot toml".into(),
                paths: vec!["Cargo.toml".into()],
                strip_path_prefixes: None,
            }),
            output: vec![
                ConfiguredProducer::Tarball(TarballProducer {
                    name: "cargo dot toml output".into(),
                    path: tar.clone(),
                    compression: compression::CompressionType::None,
                    injections: vec![Injection::Touch {
                        path: "Cargo-2.toml".into(),
                    }],
                }),
                ConfiguredProducer::File(FileProducer {
                    name: "unwrapper".into(),
                    path: tmp.path_view(),
                    preserve_empty_directories: None,
                    injections: vec![],
                }),
            ],
        };

        let pipeline = Pipeline::new();
        pipeline.run(config).await?;
        assert!(tmp.path_view().join("Cargo-2.toml").exists());
        assert!(tmp.path_view().join("Cargo.toml").exists());

        Ok(())
    }

    #[tokio::test]
    async fn test_delete_injection_works() -> Result<()> {
        let tar_dir = TempDir::new().await?;
        let tar = tar_dir.path_view().join("cargo.toml.deleteinject.tar");
        let tmp = TempDir::new().await?;

        let config = PeckishConfig {
            pipeline: true,
            input: ConfiguredArtifact::File(FileArtifact {
                name: "cargo dot toml".into(),
                paths: vec!["Cargo.toml".into()],
                strip_path_prefixes: None,
            }),
            output: vec![
                ConfiguredProducer::Tarball(TarballProducer {
                    name: "cargo dot toml output".into(),
                    path: tar.clone(),
                    compression: compression::CompressionType::None,
                    injections: vec![Injection::Delete {
                        path: "Cargo.toml".into(),
                    }],
                }),
                ConfiguredProducer::File(FileProducer {
                    name: "unwrapper".into(),
                    path: tmp.path_view(),
                    preserve_empty_directories: None,
                    injections: vec![],
                }),
            ],
        };

        let pipeline = Pipeline::new();
        pipeline.run(config).await?;
        assert!(!tmp.path_view().join("Cargo.toml").exists());

        Ok(())
    }

    #[tokio::test]
    async fn test_create_inject_works() -> Result<()> {
        let tar_dir = TempDir::new().await?;
        let tar = tar_dir.path_view().join("cargo.toml.createinject.tar");
        let tmp = TempDir::new().await?;

        let config = PeckishConfig {
            pipeline: true,
            input: ConfiguredArtifact::File(FileArtifact {
                name: "cargo dot toml".into(),
                paths: vec!["Cargo.toml".into()],
                strip_path_prefixes: None,
            }),
            output: vec![
                ConfiguredProducer::Tarball(TarballProducer {
                    name: "cargo dot toml output".into(),
                    path: tar.clone(),
                    compression: compression::CompressionType::None,
                    injections: vec![Injection::Create {
                        path: "Cargo-2.toml".into(),
                        content: "test".into(),
                    }],
                }),
                ConfiguredProducer::File(FileProducer {
                    name: "unwrapper".into(),
                    path: tmp.path_view(),
                    preserve_empty_directories: None,
                    injections: vec![],
                }),
            ],
        };

        let pipeline = Pipeline::new();
        pipeline.run(config).await?;
        assert!(tmp.path_view().join("Cargo-2.toml").exists());
        assert!(tmp.path_view().join("Cargo.toml").exists());
        let mut file = File::open(tmp.path_view().join("Cargo-2.toml")).await?;
        let mut buf = String::new();
        file.read_to_string(&mut buf).await?;
        assert_eq!(buf, "test");

        Ok(())
    }
}
