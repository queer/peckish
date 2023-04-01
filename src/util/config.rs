use std::path::PathBuf;

use color_eyre::Result;
use log::*;
use rsfs_tokio::unix_ext::GenFSExt;
use rsfs_tokio::{GenFS, Metadata};
use serde::{Deserialize, Serialize};
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::artifact::arch::{ArchArtifact, ArchProducer};
use crate::artifact::deb::{DebArtifact, DebProducer};
use crate::artifact::docker::{DockerArtifact, DockerProducer};
use crate::artifact::file::{FileArtifact, FileProducer};
use crate::artifact::tarball::{TarballArtifact, TarballProducer};
use crate::fs::MemFS;

#[derive(Debug)]
pub struct PeckishConfig {
    pub input: ConfiguredArtifact,
    pub output: Vec<ConfiguredProducer>,
    pub pipeline: bool,
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
            pipeline: config.pipeline,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct InternalConfig {
    #[serde(default)]
    pipeline: bool,
    input: InputArtifact,
    output: Vec<OutputProducer>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum InputArtifact {
    File { name: String, paths: Vec<PathBuf> },
    Tarball { name: String, path: PathBuf },
    Docker { name: String, image: String },
    Arch { name: String, path: PathBuf },
    Deb { name: String, path: PathBuf },
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

            InputArtifact::Arch { name, path } => {
                ConfiguredArtifact::Arch(ArchArtifact { name, path })
            }

            InputArtifact::Deb { name, path } => {
                ConfiguredArtifact::Deb(DebArtifact { name, path })
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
        #[serde(default)]
        injections: Vec<Injection>,
    },

    Tarball {
        name: String,
        path: PathBuf,
        #[serde(default)]
        injections: Vec<Injection>,
    },

    Docker {
        name: String,
        image: String,
        base_image: Option<String>,
        #[serde(default)]
        entrypoint: Option<Vec<String>>,
        #[serde(default)]
        injections: Vec<Injection>,
    },

    Arch {
        name: String,
        path: PathBuf,
        package_name: String,
        description: String,
        version: String,
        author: String,
        #[serde(default)]
        injections: Vec<Injection>,
    },

    Deb {
        name: String,
        path: PathBuf,
        #[serde(default)]
        prerm: Option<PathBuf>,
        #[serde(default)]
        postinst: Option<PathBuf>,
        package_name: String,
        package_maintainer: String,
        package_architecture: String,
        package_version: String,
        package_depends: String,
        package_description: String,

        #[serde(default)]
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
                base_image,
                entrypoint,
                injections,
            } => ConfiguredProducer::Docker(DockerProducer {
                name: name.clone(),
                image: image.clone(),
                base_image: base_image.clone(),
                entrypoint: entrypoint.clone(),
                injections: injections.clone(),
            }),

            OutputProducer::Arch {
                name,
                path,
                package_name,
                description,
                version,
                author,
                injections,
            } => ConfiguredProducer::Arch(ArchProducer {
                name: name.clone(),
                package_name: package_name.clone(),
                package_desc: description.clone(),
                package_ver: version.clone(),
                package_author: author.clone(),
                path: path.clone(),
                injections: injections.clone(),
            }),

            OutputProducer::Deb {
                name,
                path,
                prerm,
                postinst,
                package_name,
                package_maintainer,
                package_architecture,
                package_version,
                package_depends,
                package_description,
                injections,
            } => ConfiguredProducer::Deb(DebProducer {
                name: name.clone(),
                path: path.clone(),
                prerm: prerm.clone(),
                postinst: postinst.clone(),
                package_name: package_name.clone(),
                package_maintainer: package_maintainer.clone(),
                package_architecture: package_architecture.clone(),
                package_version: package_version.clone(),
                package_depends: package_depends.clone(),
                package_description: package_description.clone(),
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
    Arch(ArchArtifact),
    Deb(DebArtifact),
}

#[derive(Debug, Clone)]
pub enum ConfiguredProducer {
    File(FileProducer),
    Tarball(TarballProducer),
    Docker(DockerProducer),
    Arch(ArchProducer),
    Deb(DebProducer),
}

impl ConfiguredProducer {
    pub fn name(&self) -> &str {
        match self {
            ConfiguredProducer::File(producer) => &producer.name,
            ConfiguredProducer::Tarball(producer) => &producer.name,
            ConfiguredProducer::Docker(producer) => &producer.name,
            ConfiguredProducer::Arch(producer) => &producer.name,
            ConfiguredProducer::Deb(producer) => &producer.name,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Injection {
    Move { src: PathBuf, dest: PathBuf },
    Copy { src: PathBuf, dest: PathBuf },
    Symlink { src: PathBuf, dest: PathBuf },
    Touch { path: PathBuf },
    Delete { path: PathBuf },
    Create { path: PathBuf, content: Vec<u8> },
}

impl Injection {
    pub async fn inject(&self, fs: &MemFS) -> Result<()> {
        let fs = fs.as_ref();
        match self {
            Injection::Move { src, dest } => {
                debug!("moving {:?} to {:?}", src, dest);
                if let Some(parent) = dest.parent() {
                    fs.create_dir_all(parent).await?;
                    debug!("created parent: {parent:?}");
                }

                fs.rename(src, dest).await?;
            }

            Injection::Copy { src, dest } => {
                debug!("copying {:?} to {:?}", src, dest);
                fs.copy(src, dest).await?;
            }

            Injection::Symlink { src, dest } => {
                debug!("symlinking {:?} to {:?}", src, dest);
                fs.symlink(src, dest).await?;
            }

            Injection::Touch { path } => {
                debug!("touching {:?}", path);
                fs.create_dir_all(path.parent().unwrap()).await?;
                fs.create_file(path).await?;
            }

            Injection::Delete { path } => {
                debug!("deleting {:?}", path);
                let metadata = fs.metadata(path).await?;
                if metadata.is_dir() {
                    fs.remove_dir_all(path).await?;
                } else {
                    fs.remove_file(path).await?;
                }
            }

            Injection::Create { path, content } => {
                debug!("creating {:?} with content {:?}", path, content);
                fs.create_dir_all(path.parent().unwrap()).await?;
                let mut file = fs.create_file(path).await?;
                file.write_all(content).await?;
            }
        }

        Ok(())
    }
}
