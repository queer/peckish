use std::path::PathBuf;

use color_eyre::Result;
use futures_util::TryStreamExt;
use log::*;
use rsfs_tokio::unix_ext::GenFSExt;
use rsfs_tokio::GenFS;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_tar::Header;

use crate::artifact::tarball::TarballProducer;
use crate::fs::{InternalFileType, MemFS, TempDir};
use crate::util::config::Injection;
use crate::util::traverse_memfs;

use super::{Artifact, ArtifactProducer};

/// A Debian package. This is a **non-compressed** ar archive.
///
/// TODO: Preserve control files
/// TODO: Decompress more than just gzip
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
        // ar archive of:
        // /debian-binary
        // /control.tar.gz
        // /data.tar.gz
        // /debian-binary can be discarded
        // /control.tar.gz can be discarded
        // /data.tar.gz is the vfs contents
        let mut archive = ar::Archive::new(std::fs::File::open(&self.path)?);
        let fs = MemFS::new();
        self.extract_deb_to_memfs(&mut archive, &fs).await?;

        Ok(fs)
    }
}

impl DebArtifact {
    async fn extract_deb_to_memfs(
        &self,
        archive: &mut ar::Archive<std::fs::File>,
        fs: &MemFS,
    ) -> Result<()> {
        while let Some(entry) = archive.next_entry() {
            let mut ar_entry = entry?;
            let path = String::from_utf8_lossy(ar_entry.header().identifier()).to_string();
            if path == "data.tar.gz" {
                use async_compression::tokio::bufread::GzipDecoder;
                let ar_buf = {
                    use std::io::Read;
                    let mut b = vec![];
                    ar_entry.read_to_end(&mut b)?;
                    b
                };

                let reader = GzipDecoder::new(ar_buf.as_slice());
                let mut tar = tokio_tar::Archive::new(reader);
                let mut entries = tar.entries()?;
                while let Some(mut gz_entry) = entries.try_next().await? {
                    // Copy path to vfs
                    let entry_type = gz_entry.header().entry_type();
                    if entry_type.is_dir() {
                        fs.as_ref().create_dir_all(&path).await?;
                        debug!("deb: created dir: {path:#?}");
                    } else if entry_type.is_file() {
                        let mut file = fs.as_ref().create_file(&path).await?;
                        // read all bytes from entry sync
                        let mut buf = Vec::new();
                        gz_entry.read_to_end(&mut buf).await?;

                        tokio::io::copy(&mut buf.as_slice(), &mut file).await?;
                        debug!("deb: created file: {path:#?}");
                    } else if entry_type.is_symlink() {
                        let src = gz_entry.header().link_name()?.unwrap().to_path_buf();
                        let dst = PathBuf::from(path.to_string());
                        fs.as_ref().symlink(src, dst).await?;
                    }
                }
                break;
            }
        }
        Ok(())
    }
}

/// A Debian package producer. This is a **non-compressed** ar archive.
///
/// ## Caveats
///
/// - The data and control archives are **not** compressed
///
/// TODO: Support all control file features
/// TODO: Validate control file
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

    async fn produce(&self, previous: &dyn Artifact) -> Result<Self::Output> {
        let tmp = TempDir::new().await?;

        // Create data.tar from previous artifact in tmp using TarballProducer
        debug!("producing data.tar from previous artifact...");
        let data_tar = tmp.path_view().join("data.tar");
        let _tar_artifact = TarballProducer {
            name: "data.tar".to_string(),
            path: data_tar.clone(),
            injections: self.injections.clone(),
        }
        .produce(previous)
        .await?;

        // Create control.tar from control file in tmp
        debug!("producing control.tar...");
        let control_tar = tmp.path_view().join("control.tar");
        let mut control_tar_builder = tokio_tar::Builder::new(File::create(&control_tar).await?);

        // Write self.control into control.tar as /control
        let mut control_header = Header::new_gnu();
        control_header.set_entry_type(tokio_tar::EntryType::file());
        let control_data = indoc::formatdoc! {r#"
            Package: {name}
            Maintainer: {maintainer}
            Architecture: {architecture}
            Version: {version}
            Depends: {depends}
            Description: {description}
        "#,
            name = self.package_name,
            maintainer = self.package_maintainer,
            architecture = self.package_architecture,
            version = self.package_version,
            depends = self.package_depends,
            description = self.package_description,
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
        let paths = traverse_memfs(memfs, &PathBuf::from("/")).await?;
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

        Ok(DebArtifact {
            name: self.name.clone(),
            path: self.path.clone(),
        })
    }
}
