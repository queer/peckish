use std::fs::File;
use std::path::PathBuf;

use color_eyre::eyre::eyre;
use color_eyre::Result;
use deb_rust::binary::DebPackage;
use deb_rust::DebArchitecture;

use crate::fs::{MemFS, TempDir};
use crate::util::config::Injection;

use super::file::FileProducer;
use super::tarball::TarballArtifact;
use super::{Artifact, ArtifactProducer};

#[derive(Debug, Clone)]
pub struct DebArtifact {
    pub name: String,
    pub path: PathBuf,
}

#[async_trait::async_trait]
impl Artifact for DebArtifact {
    fn name(&self) -> &str {
        &self.name
    }

    async fn extract(&self) -> Result<MemFS> {
        TarballArtifact {
            name: format!("{}-tarball-extractor", self.name),
            path: self.path.clone(),
        }
        .extract()
        .await
    }
}

#[derive(Debug, Clone)]
pub struct DebProducer {
    pub name: String,
    pub package_name: String,
    pub package_ver: String,
    pub package_desc: String,
    pub package_author: String,
    pub package_arch: String,
    pub path: PathBuf,
    pub injections: Vec<Injection>,
}

#[async_trait::async_trait]
impl ArtifactProducer for DebProducer {
    type Output = DebArtifact;

    fn name(&self) -> &str {
        &self.name
    }

    fn injections(&self) -> &[Injection] {
        &self.injections
    }

    async fn produce(&self, previous: &dyn Artifact) -> Result<Self::Output> {
        let mut package = DebPackage::new(&self.package_name);

        let package_arch = match self.package_arch.as_str() {
            "amd64" => DebArchitecture::Amd64,
            "arm64" => DebArchitecture::Arm64,
            "armhf" => DebArchitecture::Armhf,
            "i386" => DebArchitecture::I386,
            "mips" => DebArchitecture::Mips,
            "mipsel" => DebArchitecture::Mipsel,
            "mips64el" => DebArchitecture::Mips64el,
            "ppc64el" => DebArchitecture::Ppc64el,
            "s390x" => DebArchitecture::S390x,
            _ => return Err(eyre!("unknown architecture {}", self.package_arch)),
        };

        package = package
            .set_version(&self.package_ver)
            .set_description(&self.package_desc)
            .set_architecture(package_arch)
            .set_maintainer(&self.package_author);

        let tmp = TempDir::new().await?;
        let _file_artifact = FileProducer {
            name: format!("{}-file-extractor", self.name),
            path: tmp.path_view(),
            injections: self.injections.clone(),
        }
        .produce(previous)
        .await?;

        package = package.with_dir(tmp.path_view(), "/".into())?;

        package.build()?.write(File::create(&self.path)?)?;

        Ok(DebArtifact {
            name: self.name.clone(),
            path: self.path.clone(),
        })
    }
}
