use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use color_eyre::Result;

use crate::util::config::Injection;
use crate::util::MemoryFS;

use super::tarball::{TarballArtifact, TarballProducer};
use super::{get_artifact_size, Artifact, ArtifactProducer};

#[derive(Debug, Clone)]
pub struct ArchArtifact {
    pub name: String,
    pub path: PathBuf,
}

#[async_trait::async_trait]
impl Artifact for ArchArtifact {
    fn name(&self) -> &str {
        &self.name
    }

    async fn extract(&self) -> Result<MemoryFS> {
        TarballArtifact {
            name: format!("{}-tarball-extractor", self.name),
            path: self.path.clone(),
        }
        .extract()
        .await
    }
}

#[derive(Debug, Clone)]
pub struct ArchProducer {
    pub name: String,
    pub package_name: String,
    pub package_ver: String,
    pub package_desc: String,
    pub package_author: String,
    pub path: PathBuf,
    pub injections: Vec<Injection>,
}

#[async_trait::async_trait]
impl ArtifactProducer for ArchProducer {
    type Output = ArchArtifact;

    fn name(&self) -> &str {
        &self.name
    }

    fn injections(&self) -> &[Injection] {
        &self.injections
    }

    async fn produce(&self, previous: &dyn Artifact) -> Result<Self::Output> {
        let size = get_artifact_size(previous).await?;

        let content = indoc::formatdoc! {r#"
            # generated by peckish
            pkgname = {0}
            pkgbase = {0}
            pkgver = {5}
            pkgdesc = {4}
            builddate = {1}
            packager = {2}
            size = {3}
            arch = x86_64
            provides = "{0}"
        "#,
            self.package_name,
            SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
            self.package_author,
            size,
            self.package_desc,
            self.package_ver
        };

        let mut new_injections = self.injections.clone();
        new_injections.push(Injection::Create {
            path: PathBuf::from(".PKGINFO"),
            content: content.clone().into(),
        });

        TarballProducer {
            name: format!("{}-tarball-producer", self.name),
            path: self.path.clone(),
            injections: new_injections,
        }
        .produce(previous)
        .await
        .map(|tarball| ArchArtifact {
            name: self.name.clone(),
            path: tarball.path,
        })
    }
}
