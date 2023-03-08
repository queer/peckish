use color_eyre::Result;
use log::*;

use crate::artifact::{Artifact, ArtifactProducer};
use crate::util::config::{ConfiguredArtifact, ConfiguredProducer, PeckishConfig};

pub struct Pipeline;

impl Pipeline {
    pub fn new() -> Self {
        Self
    }

    pub async fn run(&self, config: PeckishConfig) -> Result<()> {
        info!("running pipeline with {} steps!", config.output.len());
        let mut last_artifact: Box<dyn Artifact> = match config.input {
            ConfiguredArtifact::File(file) => Box::new(file),
            ConfiguredArtifact::Tarball(tarball) => Box::new(tarball),
            ConfiguredArtifact::Docker(docker) => Box::new(docker),
            ConfiguredArtifact::Arch(arch) => Box::new(arch),
        };

        for (i, producer) in config.output.iter().enumerate() {
            info!("step {}: {}", i + 1, producer.name());
            last_artifact = match producer {
                ConfiguredProducer::File(file) => {
                    Box::new(file.produce(last_artifact.as_ref()).await?)
                }

                ConfiguredProducer::Tarball(tarball) => {
                    Box::new(tarball.produce(last_artifact.as_ref()).await?)
                }

                ConfiguredProducer::Docker(docker) => {
                    Box::new(docker.produce(last_artifact.as_ref()).await?)
                }

                ConfiguredProducer::Arch(arch) => {
                    Box::new(arch.produce(last_artifact.as_ref()).await?)
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use color_eyre::Result;
    use tokio::fs::File;
    use tokio::io::AsyncReadExt;

    use crate::artifact::file::{FileArtifact, FileProducer};
    use crate::artifact::tarball::TarballProducer;
    use crate::fs::TempDir;
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
            input: ConfiguredArtifact::File(FileArtifact {
                name: "cargo dot toml".into(),
                paths: vec!["Cargo.toml".into()],
            }),
            output: vec![ConfiguredProducer::Tarball(TarballProducer {
                name: "cargo dot toml output".into(),
                path: tar.clone(),
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
            input: ConfiguredArtifact::File(FileArtifact {
                name: "cargo dot toml".into(),
                paths: vec!["Cargo.toml".into()],
            }),
            output: vec![
                ConfiguredProducer::Tarball(TarballProducer {
                    name: "cargo dot toml output".into(),
                    path: tar.clone(),
                    injections: vec![Injection::Move {
                        src: "Cargo.toml".into(),
                        dest: "Cargo-2.toml".into(),
                    }],
                }),
                ConfiguredProducer::File(FileProducer {
                    name: "unwrapper".into(),
                    path: tmp.path_view(),
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
            input: ConfiguredArtifact::File(FileArtifact {
                name: "cargo dot toml".into(),
                paths: vec!["Cargo.toml".into()],
            }),
            output: vec![
                ConfiguredProducer::Tarball(TarballProducer {
                    name: "cargo dot toml output".into(),
                    path: tar.clone(),
                    injections: vec![Injection::Copy {
                        src: "Cargo.toml".into(),
                        dest: "Cargo-2.toml".into(),
                    }],
                }),
                ConfiguredProducer::File(FileProducer {
                    name: "unwrapper".into(),
                    path: tmp.path_view(),
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
            input: ConfiguredArtifact::File(FileArtifact {
                name: "cargo dot toml".into(),
                paths: vec!["Cargo.toml".into()],
            }),
            output: vec![
                ConfiguredProducer::Tarball(TarballProducer {
                    name: "cargo dot toml output".into(),
                    path: tar.clone(),
                    injections: vec![Injection::Symlink {
                        src: "Cargo.toml".into(),
                        dest: "Cargo-2.toml".into(),
                    }],
                }),
                ConfiguredProducer::File(FileProducer {
                    name: "unwrapper".into(),
                    path: tmp.path_view(),
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
            input: ConfiguredArtifact::File(FileArtifact {
                name: "cargo dot toml".into(),
                paths: vec!["Cargo.toml".into()],
            }),
            output: vec![
                ConfiguredProducer::Tarball(TarballProducer {
                    name: "cargo dot toml output".into(),
                    path: tar.clone(),
                    injections: vec![Injection::Touch {
                        path: "Cargo-2.toml".into(),
                    }],
                }),
                ConfiguredProducer::File(FileProducer {
                    name: "unwrapper".into(),
                    path: tmp.path_view(),
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
            input: ConfiguredArtifact::File(FileArtifact {
                name: "cargo dot toml".into(),
                paths: vec!["Cargo.toml".into()],
            }),
            output: vec![
                ConfiguredProducer::Tarball(TarballProducer {
                    name: "cargo dot toml output".into(),
                    path: tar.clone(),
                    injections: vec![Injection::Delete {
                        path: "Cargo.toml".into(),
                    }],
                }),
                ConfiguredProducer::File(FileProducer {
                    name: "unwrapper".into(),
                    path: tmp.path_view(),
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
            input: ConfiguredArtifact::File(FileArtifact {
                name: "cargo dot toml".into(),
                paths: vec!["Cargo.toml".into()],
            }),
            output: vec![
                ConfiguredProducer::Tarball(TarballProducer {
                    name: "cargo dot toml output".into(),
                    path: tar.clone(),
                    injections: vec![Injection::Create {
                        path: "Cargo-2.toml".into(),
                        content: "test".into(),
                    }],
                }),
                ConfiguredProducer::File(FileProducer {
                    name: "unwrapper".into(),
                    path: tmp.path_view(),
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
