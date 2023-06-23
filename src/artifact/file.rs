use std::path::{Path, PathBuf};

use disk_drive::DiskDrive;
use eyre::Result;
use floppy_disk::tokio_fs::TokioFloppyDisk;
use tracing::*;

use crate::fs::MemFS;
use crate::util::config::Injection;

use super::{Artifact, ArtifactProducer, SelfBuilder, SelfValidation};

/// A path or set of paths on the filesystem.
#[derive(Debug, Clone)]
pub struct FileArtifact {
    pub name: String,
    pub paths: Vec<PathBuf>,
}

#[async_trait::async_trait]
impl Artifact for FileArtifact {
    fn name(&self) -> &str {
        &self.name
    }

    async fn extract(&self) -> Result<MemFS> {
        let fs = MemFS::new();
        let host = TokioFloppyDisk::new(None);
        debug!("copying {} paths to memfs!", self.paths.len());
        let pwd = std::env::current_dir()?;
        for path in &self.paths {
            let full_src_path = if !path.starts_with("/") {
                Path::join(&pwd, path)
            } else {
                path.to_path_buf()
            };
            debug!("copy {} -> {}", full_src_path.display(), path.display());
            DiskDrive::copy_from_src_to_dest(&host, &*fs, full_src_path, path).await?;
        }
        Ok(fs)
    }

    fn try_clone(&self) -> Result<Box<dyn Artifact>> {
        Ok(Box::new(self.clone()))
    }

    fn paths(&self) -> Option<Vec<PathBuf>> {
        Some(self.paths.clone())
    }
}

#[async_trait::async_trait]
impl SelfValidation for FileArtifact {
    async fn validate(&self) -> Result<()> {
        let mut errors = vec![];

        for path in &self.paths {
            if !path.exists() {
                errors.push(format!("path does not exist: {path:?}"));
            } else if !path.is_file() && !path.is_dir() {
                errors.push(format!("path is not a file or directory: {path:?}"));
            }
        }

        if !errors.is_empty() {
            return Err(eyre::eyre!(
                "File artifact not valid:\n{}",
                errors.join("\n")
            ));
        }

        Ok(())
    }
}

pub struct FileArtifactBuilder {
    name: String,
    paths: Vec<PathBuf>,
}

#[allow(unused)]
impl FileArtifactBuilder {
    pub fn add_path<P: Into<PathBuf>>(&mut self, path: P) -> &mut Self {
        self.paths.push(path.into());
        self
    }
}

impl SelfBuilder for FileArtifactBuilder {
    type Output = FileArtifact;

    fn new<S: Into<String>>(name: S) -> Self {
        Self {
            name: name.into(),
            paths: vec![],
        }
    }

    fn build(&self) -> Result<Self::Output> {
        Ok(FileArtifact {
            name: self.name.clone(),
            paths: self.paths.clone(),
        })
    }
}

/// Produces a set of files at the given path on the filesystem.
#[derive(Debug, Clone)]
pub struct FileProducer {
    pub name: String,
    pub path: PathBuf,
    pub preserve_empty_directories: Option<bool>,
    pub injections: Vec<Injection>,
}

#[async_trait::async_trait]
impl ArtifactProducer for FileProducer {
    type Output = FileArtifact;

    fn name(&self) -> &str {
        &self.name
    }

    fn injections(&self) -> &[Injection] {
        &self.injections
    }

    async fn produce_from(&self, previous: &dyn Artifact) -> Result<FileArtifact> {
        let mut memfs = previous.extract().await?;
        debug!("injecting memfs");
        let memfs = self.inject(&mut memfs).await?;

        if let Some(parent) = self.path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let out_disk = TokioFloppyDisk::new(Some(self.path.clone()));
        DiskDrive::copy_between(&**memfs, &out_disk).await?;
        let output_paths = nyoom::walk_ordered(&out_disk, "/").await?;

        Ok(FileArtifact {
            name: self.path.to_string_lossy().to_string(),
            paths: output_paths
                .iter()
                .map(|p| {
                    if !p.starts_with("./") {
                        Path::join(&PathBuf::from("./"), p)
                    } else {
                        p.to_path_buf()
                    }
                })
                .collect(),
        })
    }
}

#[async_trait::async_trait]
impl SelfValidation for FileProducer {
    async fn validate(&self) -> Result<()> {
        tokio::fs::create_dir_all(&self.path).await?;

        Ok(())
    }
}

pub struct FileProducerBuilder {
    name: String,
    path: PathBuf,
    preserve_empty_directories: Option<bool>,
    injections: Vec<Injection>,
}

#[allow(unused)]
impl FileProducerBuilder {
    pub fn path<P: Into<PathBuf>>(mut self, path: P) -> Self {
        self.path = path.into();
        self
    }

    pub fn preserve_empty_directories(mut self, preserve_empty_directories: bool) -> Self {
        self.preserve_empty_directories = Some(preserve_empty_directories);
        self
    }

    pub fn inject(mut self, injection: Injection) -> Self {
        self.injections.push(injection);
        self
    }
}

impl SelfBuilder for FileProducerBuilder {
    type Output = FileProducer;

    fn new<S: Into<String>>(name: S) -> Self {
        Self {
            name: name.into(),
            path: PathBuf::from("/"),
            preserve_empty_directories: None,
            injections: vec![],
        }
    }

    fn build(&self) -> Result<Self::Output> {
        Ok(FileProducer {
            name: self.name.clone(),
            path: self.path.clone(),
            preserve_empty_directories: self.preserve_empty_directories,
            injections: self.injections.clone(),
        })
    }
}
