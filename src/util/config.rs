use std::collections::HashMap;
use std::path::{Path, PathBuf};

use disk_drive::DiskDrive;
use eyre::{eyre, Result};
use floppy_disk::mem::MemOpenOptions;
use floppy_disk::tokio_fs::TokioFloppyDisk;
use floppy_disk::{FloppyDisk, FloppyMetadata, FloppyOpenOptions};
use serde::{Deserialize, Serialize};
use smoosh::CompressionType;
use tokio::fs::File;
use tokio::io::AsyncReadExt;
use tracing::*;

use crate::artifact::arch::{ArchArtifact, ArchProducer};
use crate::artifact::deb::{DebArtifact, DebProducer};
use crate::artifact::docker::{DockerArtifact, DockerProducer};
use crate::artifact::ext4::{Ext4Artifact, Ext4Producer};
use crate::artifact::file::{FileArtifact, FileProducer};
use crate::artifact::rpm::{RpmArtifact, RpmProducer};
use crate::artifact::tarball::{TarballArtifact, TarballProducer};
use crate::artifact::{Artifact, ArtifactProducer, SelfValidation};
use crate::fs::MemFS;

#[derive(Debug)]
pub struct PeckishConfig {
    pub input: ConfiguredArtifact,
    pub output: Vec<ConfiguredProducer>,
    pub chain: bool,
}

impl PeckishConfig {
    pub async fn load(config: Option<String>) -> Result<Self> {
        let config_file: PathBuf = config.unwrap_or_else(|| "./peckish.yaml".into()).into();
        info!("loading config from {}", config_file.display());
        let mut config_file = File::open(config_file).await?;
        let mut config_str = String::new();
        config_file.read_to_string(&mut config_str).await?;

        let config: InternalConfig = serde_yaml::from_str(&config_str)?;

        Ok(Self {
            input: config.input.clone().into(),
            output: config
                .output
                .iter()
                .map(|o| o.clone().convert(&config))
                .collect(),
            chain: config.chain,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PackageMetadata {
    name: String,
    version: String,
    description: String,
    author: String,
    arch: String,
    license: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct InternalConfig {
    #[serde(default)]
    chain: bool,
    metadata: PackageMetadata,
    input: InputArtifact,
    output: Vec<OutputProducer>,
    #[serde(default)]
    injections: HashMap<String, Injection>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum InputArtifact {
    File { name: String, paths: Vec<PathBuf> },
    Tarball { name: String, path: PathBuf },
    Docker { name: String, image: String },
    Arch { name: String, path: PathBuf },
    Deb { name: String, path: PathBuf },
    Rpm { name: String, path: PathBuf },
    Ext4 { name: String, path: PathBuf },
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

            InputArtifact::Arch { name, path } => ConfiguredArtifact::Arch(ArchArtifact {
                name,
                path,
                pkginfo: None,
            }),

            InputArtifact::Deb { name, path } => ConfiguredArtifact::Deb(DebArtifact {
                name,
                path,
                control: None,
                postinst: None,
                prerm: None,
            }),

            InputArtifact::Rpm { name, path } => ConfiguredArtifact::Rpm(RpmArtifact {
                name,
                path,
                spec: None,
            }),

            InputArtifact::Ext4 { name, path } => {
                ConfiguredArtifact::Ext4(Ext4Artifact { name, path })
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
        preserve_empty_directories: Option<bool>,
        #[serde(default)]
        injections: Vec<String>,
    },

    Tarball {
        name: String,
        path: PathBuf,
        compression: Option<ConfigCompression>,
        #[serde(default)]
        injections: Vec<String>,
    },

    Docker {
        name: String,
        image: String,
        base_image: Option<String>,
        #[serde(default)]
        entrypoint: Option<Vec<String>>,
        #[serde(default)]
        injections: Vec<String>,
    },

    Arch {
        name: String,
        path: PathBuf,
        #[serde(default)]
        injections: Vec<String>,
    },

    Deb {
        name: String,
        path: PathBuf,
        #[serde(default)]
        prerm: Option<PathBuf>,
        #[serde(default)]
        postinst: Option<PathBuf>,
        #[serde(default)]
        depends: String,

        #[serde(default)]
        injections: Vec<String>,
    },

    Rpm {
        name: String,
        path: PathBuf,
        #[serde(default)]
        spec: Option<String>,
        #[serde(default)]
        injections: Vec<String>,
    },

    Ext4 {
        name: String,
        path: PathBuf,
        #[serde(default)]
        injections: Vec<String>,
    },
}

// This is intended to be a one-way conversion
#[allow(clippy::from_over_into)]
impl OutputProducer {
    fn convert(&self, config: &InternalConfig) -> ConfiguredProducer {
        match self {
            OutputProducer::File {
                name,
                path,
                preserve_empty_directories,
                injections,
            } => ConfiguredProducer::File(FileProducer {
                name: name.clone(),
                path: path.clone(),
                preserve_empty_directories: *preserve_empty_directories,
                injections: injections
                    .iter()
                    .map(|i| config.injections[i].clone())
                    .collect(),
            }),

            OutputProducer::Tarball {
                name,
                path,
                compression,
                injections,
            } => ConfiguredProducer::Tarball(TarballProducer {
                name: name.clone(),
                path: path.clone(),
                compression: compression
                    .clone()
                    .unwrap_or(ConfigCompression::None)
                    .into(),
                injections: injections
                    .iter()
                    .map(|i| config.injections[i].clone())
                    .collect(),
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
                cmd: entrypoint.clone(),
                injections: injections
                    .iter()
                    .map(|i| config.injections[i].clone())
                    .collect(),
            }),

            OutputProducer::Arch {
                name,
                path,
                injections,
            } => ConfiguredProducer::Arch(ArchProducer {
                name: name.clone(),
                package_name: config.metadata.name.clone(),
                package_desc: config.metadata.description.clone(),
                package_ver: config.metadata.version.clone(),
                package_author: config.metadata.author.clone(),
                package_arch: self.convert_architecture(&config.metadata),
                path: path.clone(),
                injections: injections
                    .iter()
                    .map(|i| config.injections[i].clone())
                    .collect(),
            }),

            OutputProducer::Deb {
                name,
                path,
                prerm,
                postinst,
                depends,
                injections,
            } => ConfiguredProducer::Deb(DebProducer {
                name: name.clone(),
                path: path.clone(),
                prerm: prerm.clone(),
                postinst: postinst.clone(),
                package_name: config.metadata.name.clone(),
                package_maintainer: config.metadata.author.clone(),
                package_architecture: self.convert_architecture(&config.metadata),
                package_version: config.metadata.version.clone(),
                package_depends: depends.clone(),
                package_description: config.metadata.description.clone(),
                injections: injections
                    .iter()
                    .map(|i| config.injections[i].clone())
                    .collect(),
            }),

            OutputProducer::Rpm {
                name,
                path,
                spec: _spec,
                injections,
            } => ConfiguredProducer::Rpm(RpmProducer {
                name: name.clone(),
                path: path.clone(),
                package_name: config.metadata.name.clone(),
                package_version: config.metadata.version.clone(),
                package_license: config.metadata.license.clone(),
                package_arch: self.convert_architecture(&config.metadata),
                package_description: config.metadata.description.clone(),
                dependencies: vec![],
                injections: injections
                    .iter()
                    .map(|i| config.injections[i].clone())
                    .collect(),
            }),

            OutputProducer::Ext4 {
                name,
                path,
                injections,
            } => ConfiguredProducer::Ext4(Ext4Producer {
                name: name.clone(),
                path: path.clone(),
                injections: injections
                    .iter()
                    .map(|i| config.injections[i].clone())
                    .collect(),
            }),
        }
    }

    fn convert_architecture(&self, metadata: &PackageMetadata) -> String {
        match self {
            OutputProducer::Arch { .. } => match metadata.arch.as_str() {
                "x86_64" => "x86_64".into(),
                "amd64" => "x86_64".into(),
                "any" => "any".into(),
                _ => panic!("unsupported architecture for arch linux: {}", metadata.arch),
            },

            OutputProducer::Deb { .. } => match metadata.arch.as_str() {
                "x86_64" => "amd64".into(),
                "amd64" => "amd64".into(),
                "any" => "all".into(),
                other => other.into(),
            },

            OutputProducer::Rpm { .. } => match metadata.arch.as_str() {
                "x86_64" => "x86_64".into(),
                "amd64" => "x86_64".into(),
                "any" => "noarch".into(),
                other => other.into(),
            },

            _ => metadata.arch.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ConfigCompression {
    None,
    Bzip,
    Deflate,
    Gzip,
    Xz,
    Zlib,
    Zstd,
}

#[allow(clippy::from_over_into)]
impl Into<CompressionType> for ConfigCompression {
    fn into(self) -> CompressionType {
        match self {
            ConfigCompression::None => CompressionType::None,
            ConfigCompression::Deflate => CompressionType::Deflate,
            ConfigCompression::Gzip => CompressionType::Gzip,
            ConfigCompression::Xz => CompressionType::Xz,
            ConfigCompression::Zlib => CompressionType::Zlib,
            ConfigCompression::Zstd => CompressionType::Zstd,
            ConfigCompression::Bzip => CompressionType::Bzip,
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
    Rpm(RpmArtifact),
    Ext4(Ext4Artifact),
}

#[derive(Debug, Clone)]
pub enum ConfiguredProducer {
    File(FileProducer),
    Tarball(TarballProducer),
    Docker(DockerProducer),
    Arch(ArchProducer),
    Deb(DebProducer),
    Rpm(RpmProducer),
    Ext4(Ext4Producer),
}

// We can't make transparent enum variants or similar easily here, so this
// ugly shitfuckery happens instead.
impl ConfiguredProducer {
    pub fn name(&self) -> &str {
        match self {
            ConfiguredProducer::File(producer) => &producer.name,
            ConfiguredProducer::Tarball(producer) => &producer.name,
            ConfiguredProducer::Docker(producer) => &producer.name,
            ConfiguredProducer::Arch(producer) => &producer.name,
            ConfiguredProducer::Deb(producer) => &producer.name,
            ConfiguredProducer::Rpm(producer) => &producer.name,
            ConfiguredProducer::Ext4(producer) => &producer.name,
        }
    }

    pub async fn validate(&self) -> Result<()> {
        match self {
            ConfiguredProducer::File(producer) => producer.validate().await,
            ConfiguredProducer::Tarball(producer) => producer.validate().await,
            ConfiguredProducer::Docker(producer) => producer.validate().await,
            ConfiguredProducer::Arch(producer) => producer.validate().await,
            ConfiguredProducer::Deb(producer) => producer.validate().await,
            ConfiguredProducer::Rpm(producer) => producer.validate().await,
            ConfiguredProducer::Ext4(producer) => producer.validate().await,
        }
    }

    pub async fn produce_from(&self, previous: &dyn Artifact) -> Result<Box<dyn Artifact>> {
        match self {
            ConfiguredProducer::File(producer) => {
                Ok(producer.produce_from(previous).await.map(Box::new)?)
            }
            ConfiguredProducer::Tarball(producer) => {
                Ok(producer.produce_from(previous).await.map(Box::new)?)
            }
            ConfiguredProducer::Docker(producer) => {
                Ok(producer.produce_from(previous).await.map(Box::new)?)
            }
            ConfiguredProducer::Arch(producer) => {
                Ok(producer.produce_from(previous).await.map(Box::new)?)
            }
            ConfiguredProducer::Deb(producer) => {
                Ok(producer.produce_from(previous).await.map(Box::new)?)
            }
            ConfiguredProducer::Rpm(producer) => {
                Ok(producer.produce_from(previous).await.map(Box::new)?)
            }
            ConfiguredProducer::Ext4(producer) => {
                Ok(producer.produce_from(previous).await.map(Box::new)?)
            }
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
    HostFile { src: PathBuf, dest: PathBuf },
    HostDir { src: PathBuf, dest: PathBuf },
}

impl Injection {
    pub async fn inject(&self, memfs: &mut MemFS) -> Result<()> {
        let fs = &**memfs;
        match self {
            Injection::Move { src, dest } => {
                debug!("moving {:?} to {:?}", src, dest);
                if let Some(parent) = dest.parent() {
                    fs.create_dir_all(parent).await?;
                    debug!("created parent: {parent:?}");
                }

                Self::do_move_file(memfs, src, dest, 0).await?;
            }

            Injection::Copy { src, dest } => {
                debug!("copying {:?} to {:?}", src, dest);
                fs.copy(src, dest).await?;
            }

            Injection::Symlink { src, dest } => {
                debug!("symlinking {} to {}", dest.display(), src.display());
                fs.symlink(src, dest).await?;
            }

            Injection::Touch { path } => {
                debug!("touching {:?}", path);
                fs.create_dir_all(path.parent().unwrap()).await?;
                MemOpenOptions::new()
                    .create(true)
                    .read(true)
                    .write(true)
                    .open(fs, path)
                    .await?;
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
                fs.write(path, content).await?;
            }

            Injection::HostFile { src, dest } => {
                debug!("copying host file {:?} to {:?}", src, dest);
                let src = if src.starts_with("./") {
                    let mut out = std::env::current_dir()?;
                    out.push(src.strip_prefix("./")?);
                    out
                } else {
                    src.clone()
                };
                let host = TokioFloppyDisk::new(None);
                DiskDrive::copy_from_src_to_dest(&host, fs, src, dest).await?;
            }

            Injection::HostDir { src, dest } => {
                debug!("copying host directory {:?} to {:?}", src, dest);
                let src = if src.starts_with("./") {
                    let mut out = std::env::current_dir()?;
                    out.push(src.strip_prefix("./")?);
                    out
                } else {
                    src.clone()
                };
                let host = TokioFloppyDisk::new(None);
                DiskDrive::copy_from_src_to_dest(&host, fs, src, dest).await?;
            }
        }

        Ok(())
    }

    #[async_recursion::async_recursion]
    async fn do_move_file(memfs: &MemFS, src: &Path, dest: &Path, depth: u8) -> Result<()> {
        // if src doesn't exist in the memfs, return an error, without an exists() method
        // code goes here:
        let fs = &**memfs;
        if fs.metadata(src).await.is_err() {
            return Err(eyre!("source path {src:?} does not exist"));
        }

        if depth > 8 {
            return Err(eyre!(
                "too many symlinks (last path was {src:?} -> {dest:?})"
            ));
        }

        // possible scenarios:
        // src is file, dest is file. replace dest with src
        // src is file, dest is dir. move src into dest
        // src is file, dest is symlink. unimplemented!("resolve symlink, treat as respective case")
        // src is dir, dest is file. error
        // src is dir, dest is dir. merge src into dest
        // src is dir, dest is symlink. unimplemented!("resolve symlink, treat as respective case")
        // src is symlink, dest is file. unimplemented!("resolve symlink, treat as respective case")
        // src is symlink, dest is dir. unimplemented!("resolve symlink, treat as respective case")
        // src is symlink, dest is symlink. unimplemented!("resolve symlink, treat as respective case")
        let src_type = fs.metadata(src).await?;
        let dest_exists = fs.metadata(dest).await.is_ok();

        if dest_exists {
            let dest_type = fs.metadata(dest).await?;

            if src_type.is_file() && dest_type.is_file() {
                fs.rename(src, dest).await?;
            } else if src_type.is_file() && dest_type.is_dir() {
                let file_name = src.file_name().unwrap();
                fs.rename(src, &dest.join(file_name)).await?;
            } else if src_type.is_file() && dest_type.is_symlink() {
                let dest = memfs.resolve_symlink(dest).await?;
                Self::do_move_file(memfs, src, &dest, depth + 1).await?;
            } else if src_type.is_dir() && dest_type.is_file() {
                return Err(eyre!("cannot move directory {:?} to file {:?}", src, dest));
            } else if src_type.is_dir() && dest_type.is_dir() {
                panic!("aaaaaaaa")
            } else if src_type.is_dir() && dest_type.is_symlink() {
                let dest = memfs.resolve_symlink(dest).await?;
                Self::do_move_file(memfs, src, &dest, depth + 1).await?;
            } else if src_type.is_symlink() && dest_type.is_file() {
                let src = memfs.resolve_symlink(src).await?;
                Self::do_move_file(memfs, &src, dest, depth + 1).await?;
            } else if src_type.is_symlink() && dest_type.is_dir() {
                let src = memfs.resolve_symlink(src).await?;
                Self::do_move_file(memfs, &src, dest, depth + 1).await?;
            } else if src_type.is_symlink() && dest_type.is_symlink() {
                let src = memfs.resolve_symlink(src).await?;
                let dest = memfs.resolve_symlink(dest).await?;
                Self::do_move_file(memfs, &src, &dest, depth + 1).await?;
            } else {
                unreachable!("it should be impossible for a file to not be one of the known 3 internal types")
            }
        } else {
            if let Some(parent) = dest.parent() {
                fs.create_dir_all(parent).await?;
            }

            fs.rename(src, dest).await?;
        }

        Ok(())
    }
}
