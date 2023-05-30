use std::path::PathBuf;

use eyre::Result;
use flail::ext::facade::ExtFacadeFloppyDisk;
use tracing::*;

use crate::fs::disk::DiskDrive;
use crate::fs::MemFS;
use crate::util::config::Injection;

use super::{Artifact, ArtifactProducer, SelfBuilder, SelfValidation};

#[derive(Debug, Clone)]
pub struct Ext4Artifact {
    pub name: String,
    pub path: PathBuf,
}

#[async_trait::async_trait]
impl Artifact for Ext4Artifact {
    fn name(&self) -> &str {
        &self.name
    }

    async fn extract(&self) -> Result<MemFS> {
        let mut fs = MemFS::new();

        let floppy_disk = ExtFacadeFloppyDisk::new(&self.path)?;
        let disk_drive = DiskDrive::new();

        disk_drive.copy_between(&floppy_disk, fs.fs()).await?;

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
impl SelfValidation for Ext4Artifact {
    async fn validate(&self) -> Result<()> {
        let mut errors = vec![];

        if !self.path.exists() {
            errors.push(format!("path does not exist: {:?}", self.path));
        }

        if !self.path.is_file() {
            errors.push(format!("path is not a file: {:?}", self.path));
        }

        Ok(())
    }
}

pub struct Ext4ArtifactBuilder {
    pub name: String,
    pub path: PathBuf,
}

#[allow(unused)]
impl Ext4ArtifactBuilder {
    pub fn path<P: Into<PathBuf>>(mut self, path: P) -> Self {
        self.path = path.into();
        self
    }
}

impl SelfBuilder for Ext4ArtifactBuilder {
    type Output = Ext4Artifact;

    fn new<S: Into<String>>(name: S) -> Self {
        Self {
            name: name.into(),
            path: PathBuf::from(""),
        }
    }

    fn build(&self) -> Result<Self::Output> {
        Ok(Ext4Artifact {
            name: self.name.clone(),
            path: self.path.clone(),
        })
    }
}

#[derive(Debug, Clone)]
pub struct Ext4Producer {
    pub name: String,
    pub path: PathBuf,
    pub injections: Vec<Injection>,
}

#[async_trait::async_trait]
impl ArtifactProducer for Ext4Producer {
    type Output = Ext4Artifact;

    fn name(&self) -> &str {
        &self.name
    }

    fn injections(&self) -> &[Injection] {
        &self.injections
    }

    async fn produce_from(&self, previous: &dyn Artifact) -> Result<Ext4Artifact> {
        info!("producing {}", self.path.display());
        let mut memfs = previous.extract().await?;
        let memfs = self.inject(&mut memfs).await?;
        // we add 2M to the end *just* in case of space memes
        let size = memfs.size().await? + (1_024 * 1_024 * 1_024 * 2);

        let output = ExtFacadeFloppyDisk::create(&self.path, size)?;

        let disk_drive = DiskDrive::new();
        disk_drive.copy_between(memfs.as_ref(), &output).await?;

        Ok(Ext4Artifact {
            name: self.path.to_string_lossy().to_string(),
            path: self.path.clone(),
        })
    }
}

#[async_trait::async_trait]
impl SelfValidation for Ext4Producer {
    async fn validate(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        Ok(())
    }
}

pub struct Ext4ProducerBuilder {
    name: String,
    path: PathBuf,
    injections: Vec<Injection>,
}

#[allow(unused)]
impl Ext4ProducerBuilder {
    pub fn path<P: Into<PathBuf>>(mut self, path: P) -> Self {
        self.path = path.into();
        self
    }

    pub fn inject(mut self, injection: Injection) -> Self {
        self.injections.push(injection);
        self
    }
}

impl SelfBuilder for Ext4ProducerBuilder {
    type Output = Ext4Producer;

    fn new<S: Into<String>>(name: S) -> Self {
        Self {
            name: name.into(),
            path: PathBuf::from(""),
            injections: vec![],
        }
    }

    fn build(&self) -> Result<Self::Output> {
        Ok(Ext4Producer {
            name: self.name.clone(),
            path: self.path.clone(),
            injections: self.injections.clone(),
        })
    }
}
