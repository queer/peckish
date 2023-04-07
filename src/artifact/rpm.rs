use std::path::{Path, PathBuf};
use std::str::FromStr;

use eyre::{eyre, Result};
use log::*;
use regex::Regex;
use rsfs_tokio::mem::Permissions;
use rsfs_tokio::unix_ext::{FSMetadataExt, PermissionsExt};
use rsfs_tokio::{File, GenFS};

use crate::artifact::Artifact;
use crate::fs::{MemFS, TempDir};
use crate::util::compression;
use crate::util::config::Injection;

use super::file::FileProducer;
use super::{ArtifactProducer, SelfValidation};

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

        debug!("reading rpm: {}", &self.path.display());
        let mut rpm_file = tokio::fs::File::open(&self.path).await?;
        let metadata = rpm_file.metadata().await?;
        let size = metadata.len();
        let mut input = vec![0u8; size as usize];
        rpm_file.read_exact(&mut input).await?;
        debug!("read rpm into memory");

        let mut buf_reader = futures_util::io::BufReader::new(input.as_slice());
        let pkg = rpm::RPMPackage::parse_async(&mut buf_reader).await.unwrap();
        let fs = MemFS::new();

        let mut cpio_data = vec![];

        let join_handle = tokio::task::spawn_blocking(move || {
            compression::Context::autocompress(
                &mut pkg.content.as_slice(),
                &mut cpio_data,
                compression::CompressionType::None,
            )
            .unwrap();
            cpio_data
        });
        let cpio_data = join_handle.await?;

        debug!("building cpio reader from {} bytes", cpio_data.len());

        for file in cpio_reader::iter_files(&cpio_data) {
            let path = Path::join(Path::new("/"), Path::new(file.name()).strip_prefix(".")?);
            if let Some(parent) = path.parent() {
                fs.as_ref().create_dir_all(parent).await?;
            }

            debug!("extracting file: {:?}", path.display());
            let mut mem_file = fs.as_ref().create_file(file.name()).await?;
            let rpm_file_content = file.file().to_vec();
            let mut buf = vec![];
            let join_handle = tokio::task::spawn_blocking(move || {
                compression::Context::autocompress(
                    &mut rpm_file_content.as_slice(),
                    &mut buf,
                    compression::CompressionType::None,
                )
                .unwrap();
                buf
            });
            let buf = join_handle.await?;
            mem_file.write_all(&buf).await?;
            mem_file
                .set_permissions(Permissions::from_mode(file.mode().bits()))
                .await?;

            let uid = nix::unistd::getuid().as_raw();
            let gid = nix::unistd::getgid().as_raw();
            mem_file.chown(uid, gid).await?;
            debug!("set uid and gid to {} and {}", uid, gid);
            debug!("note: we use the current user's uid/gid for now!!! please take care!!!");
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
        .compression(rpm::Compressor::from_str("gzip").unwrap());

        for path in &file_artifact.paths {
            let rpm_path = Path::join(Path::new("/"), path.strip_prefix(tmp.path_view())?);
            debug!("writing path to rpm: {}", rpm_path.display());
            pkg = pkg
                .with_file_async(
                    path,
                    rpm::RPMFileOptions::new(rpm_path.to_string_lossy().to_string()),
                )
                .await
                .unwrap();
        }

        for dep in &self.dependencies {
            pkg = pkg.requires(rpm::Dependency::any(dep));
        }

        let pkg = pkg.build().unwrap();
        let path_clone = self.path.clone();
        let join_handle = tokio::task::spawn_blocking(move || {
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
