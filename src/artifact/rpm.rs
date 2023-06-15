use std::path::{Path, PathBuf};

use eyre::{eyre, Result};
use floppy_disk::mem::{MemOpenOptions, MemPermissions};
use floppy_disk::{
    FloppyDisk, FloppyDiskUnixExt, FloppyFile, FloppyOpenOptions, FloppyUnixPermissions,
};
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
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

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
        let fs = MemFS::new();

        let mut cpio_data = vec![];

        smoosh::recompress(
            &mut pkg.content.as_slice(),
            &mut cpio_data,
            CompressionType::None,
        )
        .await?;

        debug!("building cpio reader from {} bytes", cpio_data.len());

        for file in cpio_reader::iter_files(&cpio_data) {
            let floppy_disk = fs.as_ref();
            let path = Path::join(Path::new("/"), Path::new(file.name()).strip_prefix(".")?);
            if let Some(parent) = path.parent() {
                floppy_disk.create_dir_all(parent).await?;
            }

            debug!("extracting file: {:?}", path.display());
            let mut mem_file = MemOpenOptions::new()
                .create(true)
                .write(true)
                .open(floppy_disk, file.name())
                .await?;
            let rpm_file_content = file.file().to_vec();
            let mut buf = vec![];
            smoosh::recompress(
                &mut rpm_file_content.as_slice(),
                &mut buf,
                CompressionType::None,
            )
            .await?;
            mem_file.write_all(&buf).await?;
            mem_file
                .set_permissions(MemPermissions::from_mode(file.mode().bits()))
                .await?;

            let uid = file.uid();
            let gid = file.gid();
            floppy_disk.chown(path, uid, gid).await?;
            debug!("set uid and gid to {} and {}", uid, gid);
        }

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

    /// Produce a new artifact, given a previous artifact.
    async fn produce_from(&self, previous: &dyn Artifact) -> Result<Self::Output> {
        info!("producing {}", self.path.display());
        debug!("extracting previous artifact to tmpdir");
        let tmp = TempDir::new().await?;
        let file_artifact = FileProducer {
            name: self.name.clone(),
            path: tmp.path_view(),
            preserve_empty_directories: None,
            injections: self.injections.clone(),
        }
        .produce_from(previous)
        .await?;

        debug!("building rpm from tmpdir {}", tmp.display());
        let mut pkg = rpm::RPMBuilder::new(
            &self.package_name,
            &self.package_version,
            &self.package_license,
            &self.package_arch,
            &self.package_description,
        )
        .compression(rpm::CompressionType::Gzip);

        for path in &file_artifact.paths {
            let rpm_path = Path::join(Path::new("/"), path.strip_prefix(tmp.path_view())?);
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
        let pkg = pkg.build().unwrap();
        let path_clone = self.path.clone();
        let join_handle = tokio::task::spawn_blocking(move || {
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
