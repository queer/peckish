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
        let mut last_artifact: Box<dyn Artifact> = match config.input {
            ConfiguredArtifact::File(file) => Box::new(file),
            ConfiguredArtifact::Tarball(tarball) => Box::new(tarball),
        };

        for producer in config.output {
            debug!("running producer {}", producer.name());
            last_artifact = match producer {
                ConfiguredProducer::File(file) => {
                    Box::new(file.produce(last_artifact.as_ref()).await?)
                }
                ConfiguredProducer::Tarball(tarball) => {
                    Box::new(tarball.produce(last_artifact.as_ref()).await?)
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use color_eyre::Result;
    use tokio::fs::File;
    use tokio::io::AsyncReadExt;

    use crate::artifact::file::{FileArtifact, FileProducer};
    use crate::artifact::tarball::TarballProducer;
    use crate::util::config::Injection;

    use super::*;

    #[ctor::ctor]
    fn init() {
        crate::util::test_init();
    }

    #[tokio::test]
    async fn test_basic_pipeline_works() -> Result<()> {
        let tar = "cargo.toml.basic.tar";
        let config = PeckishConfig {
            input: ConfiguredArtifact::File(FileArtifact {
                name: "cargo dot toml".into(),
                paths: vec!["Cargo.toml".into()],
            }),
            output: vec![ConfiguredProducer::Tarball(TarballProducer {
                name: "cargo dot toml output".into(),
                path: tar.into(),
                injections: vec![],
            })],
        };

        let pipeline = Pipeline::new();
        assert!(pipeline.run(config).await.is_ok());

        tokio::fs::remove_file(tar).await?;

        Ok(())
    }

    #[tokio::test]
    async fn test_move_injection_works() -> Result<()> {
        let tar = "cargo.toml.moveinject.tar";
        let tmp = "___test-out-move___";

        let config = PeckishConfig {
            input: ConfiguredArtifact::File(FileArtifact {
                name: "cargo dot toml".into(),
                paths: vec!["Cargo.toml".into()],
            }),
            output: vec![
                ConfiguredProducer::Tarball(TarballProducer {
                    name: "cargo dot toml output".into(),
                    path: tar.into(),
                    injections: vec![Injection::Move("Cargo.toml".into(), "Cargo-2.toml".into())],
                }),
                ConfiguredProducer::File(FileProducer {
                    name: "unwrapper".into(),
                    path: tmp.into(),
                    injections: vec![],
                }),
            ],
        };

        let pipeline = Pipeline::new();
        pipeline.run(config).await?;
        assert!(Path::new(tmp).join("Cargo-2.toml").exists());
        assert!(!Path::new(tmp).join("Cargo.toml").exists());

        tokio::fs::remove_file(tar).await?;
        tokio::fs::remove_dir_all(tmp).await?;

        Ok(())
    }

    #[tokio::test]
    async fn test_copy_injection_works() -> Result<()> {
        let tar = "cargo.toml.copyinject.tar";
        let tmp = "___test-out-copy___";

        let config = PeckishConfig {
            input: ConfiguredArtifact::File(FileArtifact {
                name: "cargo dot toml".into(),
                paths: vec!["Cargo.toml".into()],
            }),
            output: vec![
                ConfiguredProducer::Tarball(TarballProducer {
                    name: "cargo dot toml output".into(),
                    path: tar.into(),
                    injections: vec![Injection::Copy("Cargo.toml".into(), "Cargo-2.toml".into())],
                }),
                ConfiguredProducer::File(FileProducer {
                    name: "unwrapper".into(),
                    path: tmp.into(),
                    injections: vec![],
                }),
            ],
        };

        let pipeline = Pipeline::new();
        pipeline.run(config).await?;
        assert!(Path::new(tmp).join("Cargo-2.toml").exists());
        assert!(Path::new(tmp).join("Cargo.toml").exists());

        tokio::fs::remove_file(tar).await?;
        tokio::fs::remove_dir_all(tmp).await?;

        Ok(())
    }

    #[tokio::test]
    async fn test_symlink_injection_works() -> Result<()> {
        let tar = "cargo.toml.symlinkinject.tar";
        let tmp = "___test-out-symlink___";

        let config = PeckishConfig {
            input: ConfiguredArtifact::File(FileArtifact {
                name: "cargo dot toml".into(),
                paths: vec!["Cargo.toml".into()],
            }),
            output: vec![
                ConfiguredProducer::Tarball(TarballProducer {
                    name: "cargo dot toml output".into(),
                    path: tar.into(),
                    injections: vec![Injection::Symlink(
                        "Cargo.toml".into(),
                        "Cargo-2.toml".into(),
                    )],
                }),
                ConfiguredProducer::File(FileProducer {
                    name: "unwrapper".into(),
                    path: tmp.into(),
                    injections: vec![],
                }),
            ],
        };

        let pipeline = Pipeline::new();
        pipeline.run(config).await?;
        assert!(Path::new(tmp).join("Cargo-2.toml").is_symlink());
        assert!(Path::new(tmp).join("Cargo.toml").exists());

        tokio::fs::remove_file(tar).await?;
        tokio::fs::remove_dir_all(tmp).await?;

        Ok(())
    }

    #[tokio::test]
    async fn test_touch_injection_works() -> Result<()> {
        let tar = "cargo.toml.touchinject.tar";
        let tmp = "___test-out-touch___";

        let config = PeckishConfig {
            input: ConfiguredArtifact::File(FileArtifact {
                name: "cargo dot toml".into(),
                paths: vec!["Cargo.toml".into()],
            }),
            output: vec![
                ConfiguredProducer::Tarball(TarballProducer {
                    name: "cargo dot toml output".into(),
                    path: tar.into(),
                    injections: vec![Injection::Touch("Cargo-2.toml".into())],
                }),
                ConfiguredProducer::File(FileProducer {
                    name: "unwrapper".into(),
                    path: tmp.into(),
                    injections: vec![],
                }),
            ],
        };

        let pipeline = Pipeline::new();
        pipeline.run(config).await?;
        assert!(Path::new(tmp).join("Cargo-2.toml").exists());
        assert!(Path::new(tmp).join("Cargo.toml").exists());

        tokio::fs::remove_file(tar).await?;
        tokio::fs::remove_dir_all(tmp).await?;

        Ok(())
    }

    #[tokio::test]
    async fn test_delete_injection_works() -> Result<()> {
        let tar = "cargo.toml.deleteinject.tar";
        let tmp = "___test-out-delete___";

        let config = PeckishConfig {
            input: ConfiguredArtifact::File(FileArtifact {
                name: "cargo dot toml".into(),
                paths: vec!["Cargo.toml".into()],
            }),
            output: vec![
                ConfiguredProducer::Tarball(TarballProducer {
                    name: "cargo dot toml output".into(),
                    path: tar.into(),
                    injections: vec![Injection::Delete("Cargo.toml".into())],
                }),
                ConfiguredProducer::File(FileProducer {
                    name: "unwrapper".into(),
                    path: tmp.into(),
                    injections: vec![],
                }),
            ],
        };

        let pipeline = Pipeline::new();
        pipeline.run(config).await?;
        assert!(!Path::new(tmp).join("Cargo.toml").exists());

        tokio::fs::remove_file(tar).await?;
        let tmp = Path::new(tmp);
        if tmp.exists() {
            tokio::fs::remove_dir_all(tmp).await?;
        }

        Ok(())
    }

    #[tokio::test]
    async fn test_create_inject_works() -> Result<()> {
        let tar = "cargo.toml.createinject.tar";
        let tmp = "___test-out-create___";

        let config = PeckishConfig {
            input: ConfiguredArtifact::File(FileArtifact {
                name: "cargo dot toml".into(),
                paths: vec!["Cargo.toml".into()],
            }),
            output: vec![
                ConfiguredProducer::Tarball(TarballProducer {
                    name: "cargo dot toml output".into(),
                    path: tar.into(),
                    injections: vec![Injection::Create("Cargo-2.toml".into(), "test".into())],
                }),
                ConfiguredProducer::File(FileProducer {
                    name: "unwrapper".into(),
                    path: tmp.into(),
                    injections: vec![],
                }),
            ],
        };

        let pipeline = Pipeline::new();
        pipeline.run(config).await?;
        assert!(Path::new(tmp).join("Cargo-2.toml").exists());
        assert!(Path::new(tmp).join("Cargo.toml").exists());
        let mut file = File::open(Path::new(tmp).join("Cargo-2.toml")).await?;
        let mut buf = String::new();
        file.read_to_string(&mut buf).await?;
        assert_eq!(buf, "test");

        tokio::fs::remove_file(tar).await?;
        tokio::fs::remove_dir_all(tmp).await?;

        Ok(())
    }
}
