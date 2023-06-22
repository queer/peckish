use std::path::{Path, PathBuf};

use disk_drive::DiskDrive;
use eyre::{eyre, Result};
use flop::cpio::CpioFloppyDisk;

use floppy_disk::tokio_fs::{TokioFloppyDisk, TokioOpenOptions};
use floppy_disk::{FloppyDisk, FloppyOpenOptions};
use regex::Regex;
use smoosh::CompressionType;
use tracing::*;

use crate::artifact::Artifact;
use crate::fs::{MemFS, TempDir};
use crate::util::config::Injection;

use super::file::FileProducer;
use super::{ArtifactProducer, SelfBuilder, SelfValidation};

#[derive(Debug, Clone)]
pub struct RpmArtifact {
    pub name: String,
    pub path: PathBuf,
    pub spec: Option<String>,
}

#[async_trait::async_trait]
impl Artifact for RpmArtifact {
    fn name(&self) -> &str {
        &self.name
    }

    async fn extract(&self) -> Result<MemFS> {
        use tokio::io::AsyncReadExt;

        info!("extracting {}...", &self.path.display());
        let mut rpm_file = tokio::fs::File::open(&self.path).await?;
        let metadata = rpm_file.metadata().await?;
        let size = metadata.len();
        let mut input = vec![0u8; size as usize];
        rpm_file.read_exact(&mut input).await?;
        debug!("read rpm into memory");

        let pkg = tokio::task::spawn_blocking(move || {
            rpm::RPMPackage::parse(&mut input.as_slice()).unwrap()
        })
        .await?;

        let tmp = TempDir::new().await?;
        let fs = MemFS::new();
        let host = TokioFloppyDisk::new(Some(tmp.path_view()));

        // Decompress cpio data to disk
        let mut host_cpio = TokioOpenOptions::new()
            .create(true)
            .write(true)
            .open(&host, "/rpm.cpio")
            .await?;

        smoosh::recompress(
            &mut pkg.content.as_slice(),
            &mut host_cpio,
            CompressionType::None,
        )
        .await?;

        let cpio = CpioFloppyDisk::open(tmp.path_view().join("rpm.cpio")).await?;
        DiskDrive::copy_between(&cpio, &*fs).await?;
        cpio.close().await?;

        debug!("done!");

        Ok(fs)
    }

    fn try_clone(&self) -> Result<Box<dyn Artifact>> {
        Ok(Box::new(RpmArtifact {
            name: self.name.clone(),
            path: self.path.clone(),
            spec: None,
        }))
    }

    fn paths(&self) -> Option<Vec<PathBuf>> {
        Some(vec![self.path.clone()])
    }
}

#[async_trait::async_trait]
impl SelfValidation for RpmArtifact {
    async fn validate(&self) -> Result<()> {
        let mut errors = vec![];

        if !self.path.exists() {
            errors.push(format!("path does not exist: {:?}", self.path));
        }

        if !self.path.is_file() {
            errors.push(format!("path is not a file: {:?}", self.path));
        }

        if !errors.is_empty() {
            return Err(eyre!("rpm artifact not valid:\n{}", errors.join("\n")));
        }

        Ok(())
    }
}

pub struct RpmArtifactBuilder {
    name: String,
    path: PathBuf,
}

#[allow(unused)]
impl RpmArtifactBuilder {
    pub fn path<S: Into<PathBuf>>(mut self, path: S) -> Self {
        self.path = path.into();
        self
    }
}

impl SelfBuilder for RpmArtifactBuilder {
    type Output = RpmArtifact;

    fn new<S: Into<String>>(name: S) -> Self {
        Self {
            name: name.into(),
            path: PathBuf::new(),
        }
    }

    fn build(&self) -> Result<Self::Output> {
        Ok(RpmArtifact {
            name: self.name.clone(),
            path: self.path.clone(),
            spec: None,
        })
    }
}

#[derive(Debug, Clone)]
pub struct RpmProducer {
    pub name: String,
    pub path: PathBuf,
    pub package_name: String,
    pub package_version: String,
    pub package_license: String,
    pub package_arch: String,
    pub package_description: String,
    pub dependencies: Vec<String>,
    pub injections: Vec<Injection>,
}

#[async_trait::async_trait]
impl ArtifactProducer for RpmProducer {
    type Output = RpmArtifact;

    fn name(&self) -> &str {
        &self.name
    }

    fn injections(&self) -> &[Injection] {
        &self.injections
    }

    async fn can_produce_from(&self, _previous: &dyn Artifact) -> Result<()> {
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
            ))?
        }
    }

    async fn produce_from(&self, previous: &dyn Artifact) -> Result<Self::Output> {
        info!("producing {}", self.path.display());
        debug!("extracting previous artifact to tmpdir");
        let tmp = TempDir::new().await?;
        FileProducer {
            name: self.name.clone(),
            path: tmp.path_view(),
            preserve_empty_directories: None,
            injections: self.injections.clone(),
        }
        .produce_from(previous)
        .await?;
        debug!("reading host files...");
        let host_dir = TokioFloppyDisk::new(Some(tmp.path_view()));
        let file_paths = nyoom::walk_ordered(&host_dir, "/").await?;

        debug!("building rpm from tmpdir {}", tmp.display());
        let mut pkg = rpm::RPMBuilder::new(
            &self.package_name,
            &self.package_version,
            &self.package_license,
            &self.package_arch,
            &self.package_description,
        )
        .compression(rpm::CompressionType::None);

        for path in &file_paths {
            let rpm_path = Path::join(Path::new("/"), path.strip_prefix(tmp.path_view())?);
            if path.is_dir() {
                debug!("skipping directory {}", path.display());
                continue;
            }
            debug!("writing path to rpm: {}", rpm_path.display());

            // TODO: This should be async... right?
            pkg = pkg
                .with_file(
                    path,
                    rpm::RPMFileOptions::new(rpm_path.to_string_lossy().to_string()),
                )
                .unwrap();
        }

        debug!("adding metadata dependencies to rpm...");
        for dep in &self.dependencies {
            pkg = pkg.requires(rpm::Dependency::any(dep));
        }

        info!("building final rpm...");
        let path_clone = self.path.clone();
        let join_handle = tokio::task::spawn_blocking(move || {
            debug!("digesting...");
            let pkg = pkg.build().unwrap();
            debug!("writing package {path_clone:?}!");
            let mut f = std::fs::File::create(path_clone).unwrap();
            pkg.write(&mut f).unwrap();
        });
        join_handle.await?;

        Ok(RpmArtifact {
            name: self.name.clone(),
            path: self.path.clone(),
            spec: None,
        })
    }
}

#[async_trait::async_trait]
impl SelfValidation for RpmProducer {
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

        if !package_version_regex.is_match(&self.package_version) {
            errors.push(format!(
                "package version `{}` is invalid, must match {package_version_regex}",
                self.package_version
            ));
        }

        if self.package_description.is_empty() {
            errors.push("package description is empty".to_string());
        }

        if !errors.is_empty() {
            Err(eyre::eyre!(
                "RPM producer is invalid:\n{}",
                errors.join("\n")
            ))?;
        }

        Ok(())
    }
}

pub struct RpmProducerBuilder {
    name: String,
    path: PathBuf,
    package_name: String,
    package_version: String,
    package_license: String,
    package_arch: String,
    package_description: String,
    dependencies: Vec<String>,
    injections: Vec<Injection>,
}

#[allow(unused)]
impl RpmProducerBuilder {
    pub fn path<S: Into<PathBuf>>(mut self, path: S) -> Self {
        self.path = path.into();
        self
    }

    pub fn package_name<S: Into<String>>(mut self, package_name: S) -> Self {
        self.package_name = package_name.into();
        self
    }

    pub fn package_version<S: Into<String>>(mut self, package_version: S) -> Self {
        self.package_version = package_version.into();
        self
    }

    pub fn package_license<S: Into<String>>(mut self, package_license: S) -> Self {
        self.package_license = package_license.into();
        self
    }

    pub fn package_arch<S: Into<String>>(mut self, package_arch: S) -> Self {
        self.package_arch = package_arch.into();
        self
    }

    pub fn package_description<S: Into<String>>(mut self, package_description: S) -> Self {
        self.package_description = package_description.into();
        self
    }

    pub fn dependency<S: Into<String>>(mut self, dependency: S) -> Self {
        self.dependencies.push(dependency.into());
        self
    }

    pub fn dependencies<S: Into<String>>(mut self, dependencies: Vec<S>) -> Self {
        self.dependencies = dependencies.into_iter().map(|d| d.into()).collect();
        self
    }

    pub fn injection(mut self, injection: Injection) -> Self {
        self.injections.push(injection);
        self
    }

    pub fn inject(mut self, injection: Injection) -> Self {
        self.injections.push(injection);
        self
    }
}

impl SelfBuilder for RpmProducerBuilder {
    type Output = RpmProducer;

    fn new<S: Into<String>>(name: S) -> Self {
        Self {
            name: name.into(),
            path: PathBuf::new(),
            package_name: String::new(),
            package_version: String::new(),
            package_license: String::new(),
            package_arch: String::new(),
            package_description: String::new(),
            dependencies: vec![],
            injections: vec![],
        }
    }

    fn build(&self) -> Result<Self::Output> {
        Ok(RpmProducer {
            name: self.name.clone(),
            path: self.path.clone(),
            package_name: self.package_name.clone(),
            package_version: self.package_version.clone(),
            package_license: self.package_license.clone(),
            package_arch: self.package_arch.clone(),
            package_description: self.package_description.clone(),
            dependencies: self.dependencies.clone(),
            injections: self.injections.clone(),
        })
    }
}
