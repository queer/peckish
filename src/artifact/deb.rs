use std::io::Cursor;
use std::path::PathBuf;

use eyre::Result;
use regex::Regex;
use rsfs_tokio::GenFS;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_tar::Header;
use tracing::*;

use crate::artifact::get_artifact_size;
use crate::artifact::tarball::TarballProducer;
use crate::fs::{InternalFileType, MemFS, TempDir};
use crate::util::config::Injection;
use crate::util::{compression, traverse_memfs};

use super::file::FileProducer;
use super::tarball::TarballArtifact;
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
        // TODO: Autodecompress the deb
        let fs = MemFS::new();
        self.extract_deb_to_memfs(&fs).await?;

        Ok(fs)
    }

    fn try_clone(&self) -> Result<Box<dyn Artifact>> {
        Ok(Box::new(self.clone()))
    }

    fn paths(&self) -> Option<Vec<PathBuf>> {
        Some(vec![self.path.clone()])
    }
}

impl DebArtifact {
    async fn extract_deb_to_memfs(&self, fs: &MemFS) -> Result<()> {
        let mut archive = {
            let path = self.path.clone();
            tokio::task::spawn_blocking(move || {
                let mut decompressed = vec![];
                let mut file = std::fs::File::open(path).unwrap();
                compression::Context::autocompress(
                    &mut file,
                    &mut decompressed,
                    compression::CompressionType::None,
                )
                .unwrap();
                ar::Archive::new(Cursor::new(decompressed))
            })
            .await?
        };
        while let Some(entry) = archive.next_entry() {
            let mut ar_entry = entry?;
            let path = String::from_utf8_lossy(ar_entry.header().identifier()).to_string();
            if path.starts_with("data.tar") {
                let ar_buf = {
                    use std::io::Read;
                    let mut b = vec![];
                    // TODO: async?
                    ar_entry.read_to_end(&mut b)?;
                    b
                };

                let produce_from_tmp = TempDir::new().await?;
                let produce_from_tarball = produce_from_tmp.path_view().join(path);
                let decompressed_tarball = produce_from_tmp.path_view().join("decompressed.tar");
                let output_tmp = TempDir::new().await?;

                // Write ar_buf to produce_from_tmp
                let mut ar_file = File::create(&produce_from_tarball).await?;
                ar_file.write_all(&ar_buf).await?;

                let decompressed_artifact = TarballProducer {
                    name: "deb data.tar decompressed".to_string(),
                    path: decompressed_tarball,
                    compression: compression::CompressionType::None,
                    injections: vec![],
                }
                .produce_from(&TarballArtifact {
                    name: "deb data.tar compressed".to_string(),
                    path: produce_from_tarball,
                })
                .await?;

                let _decompressed_data_artifact = FileProducer {
                    name: "deb data.tar decompressed files".to_string(),
                    path: output_tmp.path_view(),
                    preserve_empty_directories: Some(true),
                    injections: vec![],
                }
                .produce_from(&decompressed_artifact)
                .await?;

                // Copy decompressed_data_artifact to fs
                fs.copy_files_from_paths(
                    &vec![output_tmp.path_view()],
                    Some(output_tmp.path_view()),
                )
                .await?;

                break;
            }
        }
        Ok(())
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
        let mut archive = ar::Archive::new(std::fs::File::open(&self.path)?);
        let mut found_debian_binary = false;
        let mut found_control_tar_gz = false;
        let mut found_data_tar_gz = false;

        while let Some(entry) = archive.next_entry() {
            let ar_entry = entry?;
            let path = String::from_utf8_lossy(ar_entry.header().identifier()).to_string();
            if path == "debian-binary" {
                found_debian_binary = true;
            } else if path.contains("control.tar") {
                found_control_tar_gz = true;
            } else if path.contains("data.tar") {
                found_data_tar_gz = true;
            }
        }

        if !found_debian_binary {
            errors.push(format!(
                "deb artifact does not contain debian-binary: {:#?}",
                self.path
            ));
        }

        if !found_control_tar_gz {
            errors.push(format!(
                "deb artifact does not contain control.tar: {:#?}",
                self.path
            ));
        }

        if !found_data_tar_gz {
            errors.push(format!(
                "deb artifact does not contain data.tar: {:#?}",
                self.path
            ));
        }

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
        debug!("producing data.tar from previous artifact...");
        let data_tar = tmp.path_view().join("data.tar");
        let _tar_artifact = TarballProducer {
            name: "data.tar.gz".to_string(),
            path: data_tar.clone(),
            compression: compression::CompressionType::Gzip,
            injections: self.injections.clone(),
        }
        .produce_from(previous)
        .await?;

        // Create control.tar from control file in tmp
        debug!("producing control.tar...");
        let control_tar = tmp.path_view().join("control.tar");
        let mut control_tar_builder = tokio_tar::Builder::new(File::create(&control_tar).await?);

        // Write self.control into control.tar as /control
        let installed_size = get_artifact_size(previous).await?;
        let mut control_header = Header::new_gnu();
        control_header.set_entry_type(tokio_tar::EntryType::file());
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
        control_header.set_size(control_data.len() as u64);
        control_header.set_cksum();
        control_tar_builder
            .append_data(&mut control_header, "control", control_data.as_bytes())
            .await?;

        // Write self.prerm and self.postinst into control.tar if they exist
        if let Some(prerm) = &self.prerm {
            control_tar_builder
                .append_path_with_name(prerm, "prerm")
                .await?;
            debug!("wrote prerm file {} to control.tar", prerm.display());
        }
        if let Some(postinst) = &self.postinst {
            control_tar_builder
                .append_path_with_name(postinst, "postinst")
                .await?;
            debug!("wrote postinst file {} to control.tar", postinst.display());
        }

        // Compute the md5sums of every file in the memfs
        let mut md5sums = vec![];
        let memfs = previous.extract().await?;
        let memfs = self.inject(&memfs).await?;
        let paths = traverse_memfs(memfs, &PathBuf::from("/"), None).await?;
        for path in paths {
            if memfs.determine_file_type(&path).await? == InternalFileType::File {
                let mut file = memfs.as_ref().open_file(&path).await?;
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

        let mut md5_header = tokio_tar::Header::new_gnu();
        md5_header.set_size(md5sums.len() as u64);
        md5_header.set_entry_type(tokio_tar::EntryType::Regular);
        md5_header.set_mode(0o644);
        md5_header.set_cksum();

        control_tar_builder
            .append_data(&mut md5_header, "md5sums", &mut md5sums.as_bytes())
            .await?;
        debug!("wrote md5sums to control.tar");

        // Finish control.tar
        control_tar_builder.finish().await?;
        debug!("finished control.tar");

        // Create debian-binary in tmp
        let debian_binary = tmp.path_view().join("debian-binary");
        let mut debian_binary_file = File::create(&debian_binary).await?;
        debian_binary_file.write_all(b"2.0\n").await?;

        if let Some(parent) = self.path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        // Create .deb ar archive from debian-binary, control.tar, and data.tar
        debug!("building final .deb...");
        let mut deb_builder = ar::Builder::new(std::fs::File::create(&self.path)?);

        deb_builder.append_path(&debian_binary)?;
        deb_builder.append_path(&control_tar)?;
        deb_builder.append_path(&data_tar)?;

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
