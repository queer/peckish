use std::path::PathBuf;

use disk_drive::DiskDrive;
use eyre::Result;

use super::{Artifact, ArtifactProducer, SelfValidation};
use crate::fs::MemFS;
use crate::util::config::Injection;

#[derive(Debug, Clone)]
pub struct MemoryArtifact {
    pub name: String,
    pub fs: MemFS,
}

#[async_trait::async_trait]
impl Artifact for MemoryArtifact {
    fn name(&self) -> &str {
        &self.name
    }

    async fn extract(&self) -> Result<MemFS> {
        // TODO: Consider making .extract() take ownership and consume
        Ok(self.fs.clone())
    }

    fn try_clone(&self) -> Result<Box<dyn Artifact>> {
        Ok(Box::new(self.clone()))
    }

    fn paths(&self) -> Option<Vec<PathBuf>> {
        None
    }
}

#[async_trait::async_trait]
impl SelfValidation for MemoryArtifact {
    async fn validate(&self) -> Result<()> {
        Ok(())
    }
}

pub struct MemoryProducer {
    pub name: String,
    pub injections: Vec<Injection>,
}

#[async_trait::async_trait]
impl ArtifactProducer for MemoryProducer {
    type Output = MemoryArtifact;

    fn name(&self) -> &str {
        &self.name
    }

    fn injections(&self) -> &[Injection] {
        &self.injections
    }

    async fn produce_from(&self, previous: &dyn Artifact) -> Result<Self::Output> {
        let prev = previous.extract().await?;
        let fs = MemFS::new();
        DiskDrive::copy_between(prev.as_ref(), fs.as_ref()).await?;
        Ok(MemoryArtifact {
            name: self.name.clone(),
            fs,
        })
    }
}

#[async_trait::async_trait]
impl SelfValidation for MemoryProducer {
    async fn validate(&self) -> Result<()> {
        Ok(())
    }
}

/// An empty artifact with no files that does nothing.
#[derive(Debug, Clone)]
pub struct EmptyArtifact {
    name: String,
}

impl EmptyArtifact {
    pub fn new<S: Into<String>>(name: S) -> Self {
        Self { name: name.into() }
    }
}

#[async_trait::async_trait]
impl Artifact for EmptyArtifact {
    fn name(&self) -> &str {
        &self.name
    }

    async fn extract(&self) -> Result<MemFS> {
        Ok(MemFS::new())
    }

    fn try_clone(&self) -> Result<Box<dyn Artifact>> {
        Ok(Box::new(self.clone()))
    }

    fn paths(&self) -> Option<Vec<PathBuf>> {
        None
    }
}

#[async_trait::async_trait]
impl SelfValidation for EmptyArtifact {
    async fn validate(&self) -> Result<()> {
        Ok(())
    }
}
