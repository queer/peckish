use std::path::{Path, PathBuf};

use disk_drive::DiskDrive;
use eyre::Result;
use flop::ar::ArFloppyDisk;
use flop::tar::TarFloppyDisk;
use floppy_disk::mem::MemOpenOptions;
use floppy_disk::tokio_fs::TokioFloppyDisk;
use floppy_disk::{FloppyDisk, FloppyOpenOptions};
use regex::Regex;
use smoosh::CompressionType;
use tokio::io::AsyncReadExt;
use tracing::*;

use crate::artifact::get_artifact_size;
use crate::artifact::memory::EmptyArtifact;
use crate::artifact::tarball::{TarballProducer, TarballProducerBuilder};
use crate::fs::{InternalFileType, MemFS, TempDir};
use crate::util::config::Injection;
use crate::util::traverse_memfs;

use super::{Artifact, ArtifactProducer, SelfBuilder, SelfValidation};

/// A Debian package. This is a **non-compressed** ar archive.
#[derive(Debug, Clone)]
pub struct DebArtifact {
    pub name: String,
    pub path: PathBuf,
    pub control: Option<ControlFile>,
    pub postinst: Option<String>,
    pub prerm: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ControlFile {
    pub package: String,
    pub version: String,
    pub section: String,
    pub priority: String,
    pub architecture: String,
    pub depends: String,
    pub suggests: String,
    pub conflicts: String,
    pub replaces: String,
    pub installed_size: u64,
    pub maintainer: String,
    pub description: String,
}

#[async_trait::async_trait]
impl Artifact for DebArtifact {
    fn name(&self) -> &str {
        &self.name
    }

    async fn extract(&self) -> Result<MemFS> {
        let fs = MemFS::new();
        let tmp = TempDir::new().await?;
        let host = TokioFloppyDisk::new(Some(tmp.path_view()));
        let deb = ArFloppyDisk::open(&self.path).await?;

        let control_tar = deb
            .find_in_dir("/", "control.tar")
            .await?
            .expect("control.tar was not present in deb!?");

        let data_tar = deb
            .find_in_dir("/", "data.tar")
            .await?
            .expect("data.tar was not present in deb!?");

        DiskDrive::copy_from_src(&deb, &host, "/").await?;
        DiskDrive::copy_from_src(&deb, &host, &control_tar).await?;
        DiskDrive::copy_from_src(&deb, &host, &data_tar).await?;

        let data = TarFloppyDisk::open(tmp.path_view().join(&data_tar)).await?;
        DiskDrive::copy_between(&data, fs.as_ref()).await?;

        data.close().await?;
        deb.close().await?;

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
impl SelfValidation for DebArtifact {
    async fn validate(&self) -> Result<()> {
        let mut errors = vec![];

        if !self.path.exists() {
            errors.push(format!("deb artifact does not exist: {:#?}", self.path));
        }

        if let Some(file_name) = self.path.file_name() {
            if !file_name.to_string_lossy().to_string().ends_with(".deb") {
                errors.push(format!("{} does not end with .deb", self.path.display(),));
            }
        } else {
            errors.push(format!("{} does not have a file name", self.path.display()));
        }

        // validate that file is an ar archive of:
        // /debian-binary
        // /control.tar.gz
        // /data.tar.gz
        let deb = ArFloppyDisk::open(&self.path).await?;

        if deb.find_in_dir("/", "debian-binary").await?.is_none() {
            errors.push(format!(
                "deb artifact does not contain debian-binary: {:#?}",
                self.path
            ));
        }

        if deb.find_in_dir("/", "control.tar").await?.is_none() {
            errors.push(format!(
                "deb artifact does not contain control.tar: {:#?}",
                self.path
            ));
        }

        if deb.find_in_dir("/", "data.tar").await?.is_none() {
            errors.push(format!(
                "deb artifact does not contain data.tar: {:#?}",
                self.path
            ));
        }

        deb.close().await?;

        if !errors.is_empty() {
            return Err(eyre::eyre!(
                "Debian artifact is invalid:\n{}",
                errors.join("\n")
            ));
        }

        Ok(())
    }
}

pub struct DebArtifactBuilder {
    name: String,
    path: PathBuf,
    control: Option<ControlFile>,
    postinst: Option<String>,
    prerm: Option<String>,
}

#[allow(unused)]
impl DebArtifactBuilder {
    pub fn path<P: Into<PathBuf>>(mut self, path: P) -> Self {
        self.path = path.into();
        self
    }

    pub fn control(mut self, control: ControlFile) -> Self {
        self.control = Some(control);
        self
    }

    pub fn postinst<S: Into<String>>(mut self, postinst: S) -> Self {
        self.postinst = Some(postinst.into());
        self
    }

    pub fn prerm<S: Into<String>>(mut self, prerm: S) -> Self {
        self.prerm = Some(prerm.into());
        self
    }
}

impl SelfBuilder for DebArtifactBuilder {
    type Output = DebArtifact;

    fn new<S: Into<String>>(name: S) -> Self {
        Self {
            name: name.into(),
            path: PathBuf::new(),
            control: None,
            postinst: None,
            prerm: None,
        }
    }

    fn build(&self) -> Result<Self::Output> {
        Ok(DebArtifact {
            name: self.name.clone(),
            path: self.path.clone(),
            control: self.control.clone(),
            postinst: self.postinst.clone(),
            prerm: self.prerm.clone(),
        })
    }
}

/// A Debian package producer. This is a **non-compressed** ar archive.
///
/// ## Caveats
///
/// - The data and control archives are **not** compressed
///
/// TODO: Support all control file features
#[derive(Debug, Clone)]
pub struct DebProducer {
    pub name: String,
    pub path: PathBuf,
    pub prerm: Option<PathBuf>,
    pub postinst: Option<PathBuf>,
    pub injections: Vec<Injection>,
    pub package_name: String,
    pub package_maintainer: String,
    pub package_architecture: String,
    pub package_version: String,
    pub package_depends: String,
    pub package_description: String,
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

    async fn produce_from(&self, previous: &dyn Artifact) -> Result<Self::Output> {
        let tmp = TempDir::new().await?;

        // Create data.tar from previous artifact in tmp using TarballProducer
        info!("packaging data files...");
        debug!("producing data.tar from previous artifact...");
        let data_tar = tmp.path_view().join("data.tar.gz");
        let _tar_artifact = TarballProducer {
            name: "data.tar.gz".to_string(),
            path: data_tar.clone(),
            compression: CompressionType::Gzip,
            injections: self.injections.clone(),
        }
        .produce_from(previous)
        .await?;

        // Create control.tar from control file in tmp
        info!("packaging metadata files...");
        debug!("producing control.tar...");
        let control_tar = tmp.path_view().join("control.tar.gz");

        // Write control file to control.tar
        let installed_size = get_artifact_size(previous).await?;
        let control_data = indoc::formatdoc! {r#"
            Package: {name}
            Maintainer: {maintainer}
            Architecture: {architecture}
            Version: {version}
            Depends: {depends}
            Description: {description}
            Installed-Size: {installed_size}
        "#,
            name = self.package_name,
            maintainer = self.package_maintainer,
            architecture = self.package_architecture,
            version = self.package_version,
            depends = self.package_depends,
            description = self.package_description,
            installed_size = installed_size,
        };

        let control_tar_builder = TarballProducerBuilder::new("control.tar.gz")
            .path(control_tar.clone())
            .compression(CompressionType::Gzip)
            .inject(Injection::Create {
                path: "/control".into(),
                content: control_data.into_bytes(),
            });

        // Write self.prerm and self.postinst into control.tar if they exist
        let control_tar_builder = if let Some(prerm) = &self.prerm {
            debug!("wrote prerm file {:?} to control.tar", self.prerm);
            control_tar_builder.inject(Injection::Copy {
                src: prerm.clone(),
                dest: "/prerm".into(),
            })
        } else {
            control_tar_builder
        };
        let control_tar_builder = if let Some(postinst) = &self.postinst {
            debug!("wrote postinst file {:?} to control.tar", self.postinst);
            control_tar_builder.inject(Injection::Copy {
                src: postinst.clone(),
                dest: "/postinst".into(),
            })
        } else {
            control_tar_builder
        };

        info!("computing checksums...");
        // Compute the md5sums of every file in the memfs
        let mut md5sums = vec![];
        let mut memfs = previous.extract().await?;
        let memfs = self.inject(&mut memfs).await?;
        let paths = traverse_memfs(memfs, &PathBuf::from("/"), None).await?;
        for path in paths {
            if memfs.determine_file_type(&path).await? == InternalFileType::File {
                let mut file = MemOpenOptions::new()
                    .read(true)
                    .open(memfs.as_ref(), &path)
                    .await?;
                let mut buf = Vec::new();
                file.read_to_end(&mut buf).await?;
                let md5sum = md5::compute(buf);
                let md5sum = format!("{:x}", md5sum);
                debug!("md5sum of {}: {}", path.display(), md5sum);
                md5sums.push((path, md5sum));
            }
        }

        // Write formatted md5sums to control.tar as /md5sums
        let md5sums = md5sums
            .into_iter()
            .map(|(path, md5sum)| format!("{}  {}", md5sum, path.to_string_lossy()))
            .collect::<Vec<_>>()
            .join("\n");

        debug!("computed md5sums:\n{}", md5sums);

        let control_tar_builder = control_tar_builder.inject(Injection::Create {
            path: "/md5sums".into(),
            content: md5sums.into_bytes(),
        });
        debug!("wrote md5sums to control.tar");

        // Finish control.tar
        control_tar_builder
            .build()?
            .produce_from(&EmptyArtifact::new("control.tar"))
            .await?;
        debug!("finished control.tar");

        // Create .deb ar archive from debian-binary, control.tar, and data.tar
        info!("building final .deb...");

        let host = TokioFloppyDisk::new(Some(tmp.path_view()));
        let debfs = ArFloppyDisk::open(&self.path).await?;
        debug!("write debian-binary");
        debfs.write(Path::new("/debian-binary"), b"2.0\n").await?;
        debug!("write control tar");
        DiskDrive::copy_from_src(&host, &debfs, control_tar.file_name().unwrap()).await?;
        debug!("write data tar");
        DiskDrive::copy_from_src(&host, &debfs, data_tar.file_name().unwrap()).await?;

        debug!("done!");

        let prerm = if let Some(prerm) = &self.prerm {
            Some(tokio::fs::read_to_string(prerm).await?)
        } else {
            None
        };

        let postinst = if let Some(postinst) = &self.postinst {
            Some(tokio::fs::read_to_string(postinst).await?)
        } else {
            None
        };

        debfs.close().await?;

        Ok(DebArtifact {
            name: self.name.clone(),
            path: self.path.clone(),
            control: Some(ControlFile {
                package: self.package_name.clone(),
                maintainer: self.package_maintainer.clone(),
                architecture: self.package_architecture.clone(),
                version: self.package_version.clone(),
                depends: self.package_depends.clone(),
                description: self.package_description.clone(),
                section: "".into(),
                priority: "".into(),
                suggests: "".into(),
                conflicts: "".into(),
                replaces: "".into(),
                installed_size,
            }),
            prerm,
            postinst,
        })
    }
}

#[async_trait::async_trait]
impl SelfValidation for DebProducer {
    async fn validate(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let package_name_regex = Regex::new(r"^[a-z0-9][a-z0-9+-\.]+$")?;
        let package_maintainer_regex = Regex::new(r"^[^<]+( <[^>]+>)?$")?;
        let package_version_regex = Regex::new(r"^[a-z0-9][a-z0-9+._-]*(-\d+)$")?;

        let mut errors = vec![];

        if !package_name_regex.is_match(&self.package_name) {
            errors.push(format!(
                "package name {} is invalid, must match {package_name_regex}",
                self.package_name,
            ));
        }

        if !package_maintainer_regex.is_match(&self.package_maintainer) {
            errors.push(format!(
                "package maintainer {} is invalid, must match {package_maintainer_regex}",
                self.package_maintainer,
            ));
        }

        if !package_version_regex.is_match(&self.package_version) {
            errors.push(format!(
                "package version {} is invalid, must match {package_version_regex}",
                self.package_version,
            ));
        }

        if self.package_description.is_empty() {
            errors.push("package description must not be empty".to_string());
        }

        // validate architecture against all known debian architectures
        let valid_architectures = vec![
            "amd64", "arm64", "armel", "armhf", "i386", "mips", "mips64el", "mipsel", "ppc64el",
            "s390x", "sh4", "sh4eb", "sparc", "sparc64",
        ];

        if !valid_architectures.contains(&self.package_architecture.as_str()) {
            errors.push(format!(
                "package architecture {} is invalid, must be one of {valid_architectures:?}",
                self.package_architecture
            ));
        }

        if !errors.is_empty() {
            return Err(eyre::eyre!(
                "Debian producer is invalid:\n{}",
                errors.join("\n")
            ));
        }

        Ok(())
    }
}

pub struct DebProducerBuilder {
    name: String,
    path: PathBuf,
    prerm: Option<PathBuf>,
    postinst: Option<PathBuf>,
    injections: Vec<Injection>,
    package_name: String,
    package_maintainer: String,
    package_architecture: String,
    package_version: String,
    package_depends: String,
    package_description: String,
}

#[allow(unused)]
impl DebProducerBuilder {
    pub fn path<P: Into<PathBuf>>(mut self, path: P) -> Self {
        self.path = path.into();
        self
    }

    pub fn prerm<P: Into<PathBuf>>(mut self, prerm: P) -> Self {
        self.prerm = Some(prerm.into());
        self
    }

    pub fn postinst<P: Into<PathBuf>>(mut self, postinst: P) -> Self {
        self.postinst = Some(postinst.into());
        self
    }

    pub fn inject(mut self, injection: Injection) -> Self {
        self.injections.push(injection);
        self
    }

    pub fn package_name<S: Into<String>>(mut self, package_name: S) -> Self {
        self.package_name = package_name.into();
        self
    }

    pub fn package_maintainer<S: Into<String>>(mut self, package_maintainer: S) -> Self {
        self.package_maintainer = package_maintainer.into();
        self
    }

    pub fn package_architecture<S: Into<String>>(mut self, package_architecture: S) -> Self {
        self.package_architecture = package_architecture.into();
        self
    }

    pub fn package_version<S: Into<String>>(mut self, package_version: S) -> Self {
        self.package_version = package_version.into();
        self
    }

    pub fn package_depends<S: Into<String>>(mut self, package_depends: S) -> Self {
        self.package_depends = package_depends.into();
        self
    }

    pub fn package_description<S: Into<String>>(mut self, package_description: S) -> Self {
        self.package_description = package_description.into();
        self
    }
}

impl SelfBuilder for DebProducerBuilder {
    type Output = DebProducer;

    fn new<S: Into<String>>(name: S) -> Self {
        Self {
            name: name.into(),
            path: PathBuf::from("package.deb"),
            prerm: None,
            postinst: None,
            injections: vec![],
            package_name: "".into(),
            package_maintainer: "".into(),
            package_architecture: "".into(),
            package_version: "".into(),
            package_depends: "".into(),
            package_description: "".into(),
        }
    }

    fn build(&self) -> Result<Self::Output> {
        Ok(DebProducer {
            name: self.name.clone(),
            path: self.path.clone(),
            prerm: self.prerm.clone(),
            postinst: self.postinst.clone(),
            injections: self.injections.clone(),
            package_name: self.package_name.clone(),
            package_maintainer: self.package_maintainer.clone(),
            package_architecture: self.package_architecture.clone(),
            package_version: self.package_version.clone(),
            package_depends: self.package_depends.clone(),
            package_description: self.package_description.clone(),
        })
    }
}
