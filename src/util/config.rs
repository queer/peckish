use std::io::Write;
use std::path::PathBuf;

use color_eyre::Result;
use log::*;
use rsfs::unix_ext::GenFSExt;
use rsfs::{GenFS, Metadata};
use serde::{Deserialize, Serialize};
use tokio::fs::File;
use tokio::io::AsyncReadExt;

use crate::artifact::docker::{DockerArtifact, DockerProducer};
use crate::artifact::file::{FileArtifact, FileProducer};
use crate::artifact::tarball::{TarballArtifact, TarballProducer};

use super::MemoryFS;

#[derive(Debug)]
pub struct PeckishConfig {
    pub input: ConfiguredArtifact,
    pub output: Vec<ConfiguredProducer>,
}

impl PeckishConfig {
    pub async fn load(config: Option<String>) -> Result<Self> {
        let config_file: PathBuf = config.unwrap_or_else(|| "peckish.yaml".into()).into();
        let mut config_file = File::open(config_file).await?;
        let mut config_str = String::new();
        config_file.read_to_string(&mut config_str).await?;

        let config: InternalConfig = serde_yaml::from_str(&config_str)?;

        Ok(Self {
            input: config.input.into(),
            output: config.output.iter().map(|o| o.into()).collect(),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct InternalConfig {
    input: InputArtifact,
    output: Vec<OutputProducer>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum InputArtifact {
    File { name: String, paths: Vec<PathBuf> },
    Tarball { name: String, path: PathBuf },
    Docker { name: String, image: String },
}

// Safety: This is intended to be a one-way conversion
#[allow(clippy::from_over_into)]
impl Into<ConfiguredArtifact> for InputArtifact {
    fn into(self) -> ConfiguredArtifact {
        match self {
            InputArtifact::File { name, paths } => {
                ConfiguredArtifact::File(FileArtifact { name, paths })
            }

            InputArtifact::Tarball { name, path } => {
                ConfiguredArtifact::Tarball(TarballArtifact { name, path })
            }

            InputArtifact::Docker { name, image } => {
                ConfiguredArtifact::Docker(DockerArtifact { name, image })
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum OutputProducer {
    File {
        name: String,
        path: PathBuf,
        injections: Vec<Injection>,
    },
    Tarball {
        name: String,
        path: PathBuf,
        injections: Vec<Injection>,
    },
    Docker {
        name: String,
        image: String,
        injections: Vec<Injection>,
    },
}

// Safety: This is intended to be a one-way conversion
#[allow(clippy::from_over_into)]
impl Into<ConfiguredProducer> for &OutputProducer {
    fn into(self) -> ConfiguredProducer {
        match self {
            OutputProducer::File {
                name,
                path,
                injections,
            } => ConfiguredProducer::File(FileProducer {
                name: name.clone(),
                path: path.clone(),
                injections: injections.clone(),
            }),

            OutputProducer::Tarball {
                name,
                path,
                injections,
            } => ConfiguredProducer::Tarball(TarballProducer {
                name: name.clone(),
                path: path.clone(),
                injections: injections.clone(),
            }),

            OutputProducer::Docker {
                name,
                image,
                injections,
            } => ConfiguredProducer::Docker(DockerProducer {
                name: name.clone(),
                image: image.clone(),
                injections: injections.clone(),
            }),
        }
    }
}

#[derive(Debug, Clone)]
pub enum ConfiguredArtifact {
    File(FileArtifact),
    Tarball(TarballArtifact),
    Docker(DockerArtifact),
}

#[derive(Debug, Clone)]
pub enum ConfiguredProducer {
    File(FileProducer),
    Tarball(TarballProducer),
    Docker(DockerProducer),
}

impl ConfiguredProducer {
    pub fn name(&self) -> &str {
        match self {
            ConfiguredProducer::File(producer) => &producer.name,
            ConfiguredProducer::Tarball(producer) => &producer.name,
            ConfiguredProducer::Docker(producer) => &producer.name,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Injection {
    Move(PathBuf, PathBuf),
    Copy(PathBuf, PathBuf),
    Symlink(PathBuf, PathBuf),
    Touch(PathBuf),
    Delete(PathBuf),
    Create(PathBuf, Vec<u8>),
}

impl Injection {
    pub fn inject(&self, fs: &MemoryFS) -> Result<()> {
        match self {
            Injection::Move(src, dest) => {
                debug!("Moving {:?} to {:?}", src, dest);
                fs.rename(src, dest)?;
            }
            Injection::Copy(src, dest) => {
                debug!("Copying {:?} to {:?}", src, dest);
                fs.copy(src, dest)?;
            }
            Injection::Symlink(src, dest) => {
                debug!("Symlinking {:?} to {:?}", src, dest);
                fs.symlink(src, dest)?;
            }
            Injection::Touch(path) => {
                debug!("Touching {:?}", path);
                fs.create_dir_all(path.parent().unwrap())?;
                fs.create_file(path)?;
            }
            Injection::Delete(path) => {
                debug!("Deleting {:?}", path);
                let metadata = fs.metadata(path)?;
                if metadata.is_dir() {
                    fs.remove_dir_all(path)?;
                } else {
                    fs.remove_file(path)?;
                }
            }
            Injection::Create(path, content) => {
                debug!("Creating {:?} with content {:?}", path, content);
                fs.create_dir_all(path.parent().unwrap())?;
                let mut file = fs.create_file(path)?;
                file.write_all(content)?;
            }
        }

        Ok(())
    }
}
