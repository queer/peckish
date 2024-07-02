use std::path::PathBuf;

use disk_drive::DiskDrive;
use eyre::eyre;
use eyre::Result;
use flop::tar::TarFloppyDisk;
use floppy_disk::tokio_fs::TokioFloppyDisk;
use floppy_disk::FloppyDisk;
use smoosh::CompressionType;
use tokio::fs::File;
use tokio::fs::OpenOptions;
use tracing::*;

use crate::fs::MemFS;
use crate::util::config::Injection;

use super::{Artifact, ArtifactProducer, SelfBuilder, SelfValidation};

/// A tarball on the filesystem at the given path.
#[derive(Debug, Clone)]
pub struct TarballArtifact {
    pub name: String,
    pub path: PathBuf,
}

#[async_trait::async_trait]
impl Artifact for TarballArtifact {
    fn name(&self) -> &str {
        &self.name
    }

    async fn extract(&self) -> Result<MemFS> {
        let fs = MemFS::new();

        info!("unpacking {}", self.path.display());
        let tarball = TarFloppyDisk::open(&self.path).await.unwrap();
        DiskDrive::copy_between(&tarball, &*fs).await?;

        Ok(fs)
    }

    fn try_clone(&self) -> Result<Box<dyn Artifact>> {
        Ok(Box::new(self.clone()))
    }

    fn paths(&self) -> Option<Vec<PathBuf>> {
        Some(vec![self.path.clone()])
    }
}

#[async_trait::async_trait]
impl SelfValidation for TarballArtifact {
    async fn validate(&self) -> Result<()> {
        let mut errors = vec![];

        if !self.path.exists() {
            errors.push(format!("path does not exist: {:?}", self.path));
        }

        if !self.path.is_file() {
            errors.push(format!("path is not a file: {:?}", self.path));
        }

        if !errors.is_empty() {
            return Err(eyre!("tarball artifact not valid:\n{}", errors.join("\n")));
        }

        Ok(())
    }
}

pub struct TarballArtifactBuilder {
    pub name: String,
    pub path: PathBuf,
}

#[allow(unused)]
impl TarballArtifactBuilder {
    pub fn path<P: Into<PathBuf>>(mut self, path: P) -> Self {
        self.path = path.into();
        self
    }
}

impl SelfBuilder for TarballArtifactBuilder {
    type Output = TarballArtifact;

    fn new<S: Into<String>>(name: S) -> Self {
        Self {
            name: name.into(),
            path: PathBuf::from(""),
        }
    }

    fn build(&self) -> Result<Self::Output> {
        Ok(TarballArtifact {
            name: self.name.clone(),
            path: self.path.clone(),
        })
    }
}

/// Produces a tarball at the given path on the filesystem.
#[derive(Debug, Clone)]
pub struct TarballProducer {
    pub name: String,
    pub path: PathBuf,
    pub compression: CompressionType,
    pub injections: Vec<Injection>,
}

#[async_trait::async_trait]
impl ArtifactProducer for TarballProducer {
    type Output = TarballArtifact;

    fn name(&self) -> &str {
        &self.name
    }

    fn injections(&self) -> &[Injection] {
        &self.injections
    }

    async fn produce_from(&self, previous: &dyn Artifact) -> Result<TarballArtifact> {
        info!("producing {}", self.path.display());
        let mut memfs = previous.extract().await?;
        self.inject(&mut memfs).await?;

        if let Some(parent) = self.path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let tarball = TarFloppyDisk::open(&self.path).await?;
        DiskDrive::copy_between(&*memfs, &tarball).await?;
        tarball.close().await?;

        // Compress tarball with smoosh
        let mut built_tarball = File::open(&self.path).await?;
        let compressed_path = {
            let mut path = self.path.clone();
            let file_name = path.clone();
            let file_name = file_name
                .file_name()
                .expect("tarball filename not present!?");
            path.pop();
            path.push(format!(
                "{}.compressed",
                file_name
                    .to_str()
                    .expect("tarball filename not valid string?!")
            ));
            path
        };
        let mut compressed_tarball = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&compressed_path)
            .await?;
        smoosh::recompress(
            &mut built_tarball,
            &mut compressed_tarball,
            self.compression,
        )
        .await?;
        tokio::fs::remove_file(&self.path).await?;
        tokio::fs::rename(&compressed_path, &self.path).await?;

        Ok(TarballArtifact {
            name: self.path.to_string_lossy().to_string(),
            path: self.path.clone(),
        })
    }
}

#[async_trait::async_trait]
impl SelfValidation for TarballProducer {
    async fn validate(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        if TokioFloppyDisk::new(None)
            .metadata(&self.path)
            .await
            .is_err()
        {
            Ok(())
        } else {
            Err(eyre::eyre!(
                "cannot produce artifact '{}': path already exists: {}",
                self.name,
                self.path.display()
            ))
        }
    }
}

pub struct TarballProducerBuilder {
    name: String,
    path: PathBuf,
    compression: CompressionType,
    injections: Vec<Injection>,
}

#[allow(unused)]
impl TarballProducerBuilder {
    pub fn path<P: Into<PathBuf>>(mut self, path: P) -> Self {
        self.path = path.into();
        self
    }

    pub fn compression(mut self, compression: CompressionType) -> Self {
        self.compression = compression;
        self
    }

    pub fn inject(mut self, injection: Injection) -> Self {
        self.injections.push(injection);
        self
    }
}

impl SelfBuilder for TarballProducerBuilder {
    type Output = TarballProducer;

    fn new<S: Into<String>>(name: S) -> Self {
        Self {
            name: name.into(),
            path: PathBuf::from(""),
            compression: CompressionType::None,
            injections: vec![],
        }
    }

    fn build(&self) -> Result<Self::Output> {
        Ok(TarballProducer {
            name: self.name.clone(),
            path: self.path.clone(),
            compression: self.compression,
            injections: self.injections.clone(),
        })
    }
}
