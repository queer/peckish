use std::path::PathBuf;

use color_eyre::eyre::eyre;
use color_eyre::Result;
use log::*;
use rsfs_tokio::unix_ext::PermissionsExt;
use rsfs_tokio::{GenFS, Metadata};
use tokio::fs::File;
use tokio_tar::{Archive, EntryType, Header};

use crate::fs::{InternalFileType, MemFS, TempDir};
use crate::util::config::Injection;
use crate::util::{traverse_memfs, Fix};

use super::{Artifact, ArtifactProducer};

#[derive(Debug, Clone)]
pub struct TarballArtifact {
    pub name: String,
    pub path: PathBuf,
}

#[async_trait::async_trait]
impl Artifact for TarballArtifact {
    fn name(&self) -> &str {
        &self.name
    }

    async fn extract(&self) -> Result<MemFS> {
        let fs = MemFS::new();

        // Unpack TAR to a temporary archive, then copy it to the memory
        // filesystem.
        // This is sadly necessary because Rust's tar libraries don't allow for
        // in-memory manipulation.
        debug!("unpacking tarball to {:?}", self.path);
        let mut archive = Archive::new(File::open(&self.path).await.map_err(Fix::Io)?);
        let tmp = TempDir::new().await?;
        debug!(
            "unpacking archive to temporary directory: {:?}",
            tmp.path_view()
        );
        archive.unpack(&tmp).await.map_err(Fix::Io)?;
        let mut walk_results = tokio::fs::read_dir(tmp.path_view()).await?;
        let mut paths = vec![];
        while let Some(path) = walk_results.next_entry().await? {
            paths.push(path.path());
        }

        debug!("copying {} paths to memfs!", paths.len());

        fs.copy_files_from_paths(&paths, Some(tmp.path_view()))
            .await?;

        Ok(fs)
    }
}

#[derive(Debug, Clone)]
pub struct TarballProducer {
    pub name: String,
    pub path: PathBuf,
    pub injections: Vec<Injection>,
}

#[async_trait::async_trait]
impl ArtifactProducer for TarballProducer {
    type Output = TarballArtifact;

    fn name(&self) -> &str {
        &self.name
    }

    fn injections(&self) -> &[Injection] {
        &self.injections
    }

    async fn produce(&self, previous: &dyn Artifact) -> Result<TarballArtifact> {
        let memfs = previous.extract().await?;
        let memfs = self.inject(&memfs).await?;
        let paths = traverse_memfs(memfs, &PathBuf::from("/")).await?;

        if let Some(parent) = self.path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let file = File::create(&self.path).await.map_err(Fix::Io)?;
        let mut archive_builder = tokio_tar::Builder::new(file);
        archive_builder.follow_symlinks(false);
        for path in paths {
            debug!("tarball producing path: {path:?}");
            let path = path.strip_prefix("/")?;

            // We use ustar headers because long paths get weird w/ gnu
            let mut header = Header::new_ustar();
            header.set_path(path).map_err(Fix::Io)?;

            let file_type = memfs.determine_file_type(path).await?;
            let fs = memfs.as_ref();
            if file_type == InternalFileType::Dir {
                let metadata = fs.metadata(path).await?;
                header.set_entry_type(EntryType::Directory);
                header.set_size(0);
                header.set_mode(metadata.permissions().mode());
                header.set_cksum();

                debug!("copy dir {path:?} with perms: {:o}", header.mode()?);

                let empty: &[u8] = &[];
                archive_builder.append(&header, empty).await?;
            } else if file_type == InternalFileType::File {
                use rsfs_tokio::File;

                let mut data = Vec::new();
                let mut stream = fs.open_file(path).await?;
                tokio::io::copy(&mut stream, &mut data).await?;

                header.set_entry_type(EntryType::Regular);
                header.set_size(data.len() as u64);
                header.set_mode(stream.metadata().await?.permissions().mode());
                header.set_cksum();

                debug!("copy file {path:?} with perms: {:o}", header.mode()?);

                archive_builder
                    .append_data(&mut header, path, data.as_slice())
                    .await
                    .map_err(Fix::Io)?;
            } else if file_type == InternalFileType::Symlink {
                let link = fs.read_link(path).await?;
                let empty: &[u8] = &[];

                header.set_entry_type(EntryType::Symlink);
                header.set_link_name(link.to_str().unwrap())?;
                header.set_size(empty.len() as u64);
                header.set_cksum();

                debug!("copy symlink {path:?}");

                archive_builder.append(&header, empty).await?;
            } else {
                return Err(eyre!("Unsupported file type: {:?}", file_type));
            }
        }

        archive_builder.into_inner().await?;

        Ok(TarballArtifact {
            name: self.path.to_string_lossy().to_string(),
            path: self.path.clone(),
        })
    }
}
