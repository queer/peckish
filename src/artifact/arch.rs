use std::path::PathBuf;

use eyre::Result;
use regex::Regex;
use tokio::fs::File;
use tokio_stream::StreamExt;

use crate::fs::MemFS;
use crate::util::config::Injection;
use crate::util::{self, compression};

use super::tarball::{TarballArtifact, TarballProducer};
use super::{get_artifact_size, Artifact, ArtifactProducer, SelfBuilder, SelfValidation};

/// An Arch Linux package. This is a tarball file with a `.pkg.tar` extension
/// and a `.PKGINFO` file in the root.
#[derive(Debug, Clone)]
pub struct ArchArtifact {
    /// The name of the artifact. Used for ex. logging.
    pub name: String,
    /// The path to the artifact.
    pub path: PathBuf,
    pub pkginfo: Option<Pkginfo>,
}

#[derive(Debug, Clone)]
pub struct Pkginfo {
    pub pkgname: String,
    pub pkgbase: String,
    pub pkgver: String,
    pub pkgdesc: String,
    pub builddate: u64,
    pub packager: String,
    pub size: u64,
    pub arch: String,
    pub provides: Vec<String>,
}

#[async_trait::async_trait]
impl Artifact for ArchArtifact {
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

    fn try_clone(&self) -> Result<Box<dyn Artifact>> {
        Ok(Box::new(self.clone()))
    }

    fn paths(&self) -> Option<Vec<PathBuf>> {
        Some(vec![self.path.clone()])
    }
}

#[async_trait::async_trait]
impl SelfValidation for ArchArtifact {
    async fn validate(&self) -> Result<()> {
        let mut errors = vec![];

        if !self.path.exists() {
            errors.push(format!("{} does not exist", self.path.display()));
        }

        if !self.path.is_file() {
            errors.push(format!("{} is not a file", self.path.display()));
        }

        if let Some(file_name) = self.path.file_name() {
            if !file_name
                .to_string_lossy()
                .to_string()
                .ends_with(".pkg.tar")
            {
                errors.push(format!(
                    "{} does not end with .pkg.tar",
                    self.path.display(),
                ));
            }
        } else {
            errors.push(format!("{} does not have a file name", self.path.display()));
        }

        // Validate that the .PKGINFO file exists in the tarball
        let mut tarball = tokio_tar::Archive::new(File::open(&self.path).await?);
        let mut pkginfo_exists = false;

        let mut entries = tarball.entries()?;
        while let Some(gz_entry) = entries.try_next().await? {
            let path = gz_entry.path()?;
            if path.ends_with(".PKGINFO") {
                pkginfo_exists = true;
                break;
            }
        }

        if !pkginfo_exists {
            errors.push(format!(
                "{} does not contain a .PKGINFO file",
                self.path.display()
            ));
        }

        if !errors.is_empty() {
            Err(eyre::eyre!(
                "Arch artifact is invalid:\n{}",
                errors.join("\n")
            ))?;
        }

        Ok(())
    }
}

pub struct ArchArtifactBuilder {
    pub name: String,
    pub path: PathBuf,
    pub pkginfo: Option<Pkginfo>,
}

#[allow(unused)]
impl ArchArtifactBuilder {
    pub fn path<P: Into<PathBuf>>(mut self, path: P) -> Self {
        self.path = path.into();
        self
    }

    pub fn pkginfo(mut self, pkginfo: Pkginfo) -> Self {
        self.pkginfo = Some(pkginfo);
        self
    }
}

impl SelfBuilder for ArchArtifactBuilder {
    type Output = ArchArtifact;

    fn new<S: Into<String>>(name: S) -> Self {
        Self {
            name: name.into(),
            path: PathBuf::new(),
            pkginfo: None,
        }
    }

    fn build(&self) -> Result<Self::Output> {
        Ok(ArchArtifact {
            name: self.name.clone(),
            path: self.path.clone(),
            pkginfo: self.pkginfo.clone(),
        })
    }
}

/// An [`ArtifactProducer`] that produces an Arch Linux package. This is a
/// tarball with a `.pkg.tar` extension and a `.PKGINFO` file in the root. The
/// `.PKGINFO` file is generated from the `package_*` fields on the struct.
/// The size of the package is calculated from the previous artifact's memfs.
#[derive(Debug, Clone)]
pub struct ArchProducer {
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
impl ArtifactProducer for ArchProducer {
    type Output = ArchArtifact;

    fn name(&self) -> &str {
        &self.name
    }

    fn injections(&self) -> &[Injection] {
        &self.injections
    }

    async fn produce_from(&self, previous: &dyn Artifact) -> Result<Self::Output> {
        let size = get_artifact_size(previous).await?;
        let builddate = util::get_current_time()?;

        let content = indoc::formatdoc! {r#"
            # generated by peckish
            pkgname = {name}
            pkgbase = {name}
            pkgver = {version}
            pkgdesc = {desc}
            builddate = {time}
            packager = {author}
            size = {size}
            arch = {arch}
            provides = {name}
        "#,
            name = self.package_name,
            time = builddate,
            author = self.package_author,
            size = size,
            desc = self.package_desc,
            version = self.package_ver,
            arch = self.package_arch,
        };

        let mut new_injections = self.injections.clone();
        new_injections.push(Injection::Create {
            path: PathBuf::from(".PKGINFO"),
            content: content.clone().into(),
        });

        if let Some(parent) = self.path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        TarballProducer {
            name: format!("{}-tarball-producer", self.name),
            path: self.path.clone(),
            compression: compression::CompressionType::Zstd,
            injections: new_injections,
        }
        .produce_from(previous)
        .await
        .map(|tarball| ArchArtifact {
            name: self.name.clone(),
            path: tarball.path,
            pkginfo: Some(Pkginfo {
                pkgname: self.package_name.clone(),
                pkgbase: self.package_name.clone(),
                pkgver: self.package_ver.clone(),
                pkgdesc: self.package_desc.clone(),
                builddate,
                packager: self.package_author.clone(),
                size,
                arch: self.package_arch.clone(),
                provides: vec![self.package_name.clone()],
            }),
        })
    }
}

#[async_trait::async_trait]
impl SelfValidation for ArchProducer {
    async fn validate(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let mut errors = vec![];

        // Validate any package starting with a letter, followed by any letter,
        // number, hyphen, or underscore, ending with a letter or number.
        let package_name_regex = Regex::new(r"^[a-z]([a-z0-9_-]*[a-z0-9])?$")?;

        // Validate more/less every version number people are likely to use,
        // and ensure it ends with the Arch-specific versioning number at the
        // end.
        let package_version_regex = Regex::new(r"^[a-z0-9][a-z0-9+._-]*(-\d+)$")?;

        if !package_name_regex.is_match(&self.package_name) {
            errors.push(format!(
                "package name `{}` is invalid, must match {package_name_regex}",
                self.package_name
            ));
        }

        if !package_version_regex.is_match(&self.package_ver) {
            errors.push(format!(
                "package version `{}` is invalid, must match {package_version_regex}",
                self.package_ver
            ));
        }

        if self.package_desc.is_empty() {
            errors.push("package description is empty".to_string());
        }

        if self.package_author.is_empty() {
            errors.push("package author is empty".to_string());
        }

        // https://wiki.archlinux.org/title/Arch_package_guidelines#Architectures
        if self.package_arch != "any" && self.package_arch != "x86_64" {
            errors.push(format!(
                "package architecture `{}` is invalid, must be one of: x86_64, any",
                self.package_arch
            ));
        }

        if !errors.is_empty() {
            Err(eyre::eyre!(
                "Arch producer is invalid:\n{}",
                errors.join("\n")
            ))?;
        }

        Ok(())
    }
}

pub struct ArchProducerBuilder {
    name: String,
    package_name: String,
    package_ver: String,
    package_desc: String,
    package_author: String,
    package_arch: String,
    path: PathBuf,
    injections: Vec<Injection>,
}

#[allow(unused)]
impl ArchProducerBuilder {
    pub fn package_name<S: Into<String>>(mut self, package_name: S) -> Self {
        self.package_name = package_name.into();
        self
    }

    pub fn package_ver<S: Into<String>>(mut self, package_ver: S) -> Self {
        self.package_ver = package_ver.into();
        self
    }

    pub fn package_desc<S: Into<String>>(mut self, package_desc: S) -> Self {
        self.package_desc = package_desc.into();
        self
    }

    pub fn package_author<S: Into<String>>(mut self, package_author: S) -> Self {
        self.package_author = package_author.into();
        self
    }

    pub fn package_arch<S: Into<String>>(mut self, package_arch: S) -> Self {
        self.package_arch = package_arch.into();
        self
    }

    pub fn path<P: Into<PathBuf>>(mut self, path: P) -> Self {
        self.path = path.into();
        self
    }

    pub fn inject(mut self, injection: Injection) -> Self {
        self.injections.push(injection);
        self
    }
}

impl SelfBuilder for ArchProducerBuilder {
    type Output = ArchProducer;

    fn new<S: Into<String>>(name: S) -> Self {
        Self {
            name: name.into(),
            package_name: String::new(),
            package_ver: String::new(),
            package_desc: String::new(),
            package_author: String::new(),
            package_arch: String::new(),
            path: PathBuf::new(),
            injections: vec![],
        }
    }

    fn build(&self) -> Result<Self::Output> {
        Ok(ArchProducer {
            name: self.name.clone(),
            package_name: self.package_name.clone(),
            package_ver: self.package_ver.clone(),
            package_desc: self.package_desc.clone(),
            package_author: self.package_author.clone(),
            package_arch: self.package_arch.clone(),
            path: self.path.clone(),
            injections: self.injections.clone(),
        })
    }
}
