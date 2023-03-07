use std::collections::HashMap;
use std::path::PathBuf;

use color_eyre::eyre::eyre;
use color_eyre::Result;
use log::*;
use rsfs::GenFS;
use tokio::fs::File;
use tokio_tar::{Archive, EntryType, Header};

use crate::util::config::Injection;
use crate::util::{create_tmp_dir, traverse_memfs, Fix, MemoryFS};

use super::{Artifact, ArtifactProducer, InternalFileType};

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

    fn description(&self) -> &str {
        "An artifact of one or more files"
    }

    async fn extract(&self) -> Result<MemoryFS> {
        let fs = MemoryFS::new();

        // Unpack TAR to a temporary archive, then copy it to the memory
        // filesystem.
        // This is sadly necessary because Rust's tar libraries don't allow for
        // in-memory manipulation.
        debug!("unpacking tarball to {:?}", self.path);
        let mut archive = Archive::new(File::open(&self.path).await.map_err(Fix::Io)?);
        let tmp = create_tmp_dir().await?;
        debug!("unpacking archive to temporary directory: {:?}", tmp);
        archive.unpack(&tmp).await.map_err(Fix::Io)?;
        let walk_results = nyoom::walk(&tmp, |_path, _| ())?;
        let paths = walk_results
            .paths
            .iter()
            .map(|e| {
                let path = e.key().clone();
                let memfs_path = path.strip_prefix(&tmp).unwrap().to_path_buf();
                (path, memfs_path)
            })
            .filter(|(_, memfs_path)| !memfs_path.as_os_str().is_empty())
            .collect::<HashMap<_, _>>();

        debug!("copying {} paths to memfs!", paths.len());
        super::copy_files_from_paths_to_memfs(&paths, &fs).await?;

        if tmp.exists() {
            debug!("removing temporary directory: {:?}", tmp);
            tokio::fs::remove_dir_all(tmp).await.map_err(Fix::Io)?;
        }

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
        let fs = previous.extract().await?;
        let fs = self.inject(&fs)?;
        let paths = traverse_memfs(fs, &PathBuf::from("/"))?;

        let file = File::create(&self.path).await.map_err(Fix::Io)?;
        let mut archive_builder = tokio_tar::Builder::new(file);
        archive_builder.follow_symlinks(false);
        for path in paths {
            debug!("tarball producing path: {path:?}");
            let path = path.strip_prefix("/")?;

            let mut header = Header::new_gnu();
            header.set_path(path).map_err(Fix::Io)?;

            let file_type = super::determine_file_type_from_memfs(fs, path)?;
            if file_type == InternalFileType::Dir {
                header.set_entry_type(EntryType::Directory);
                header.set_size(0);
                header.set_cksum();

                let empty: &[u8] = &[];
                archive_builder.append(&header, empty).await?;
            } else if file_type == InternalFileType::File {
                let mut data = Vec::new();
                let mut stream = fs.open_file(path)?;
                std::io::copy(&mut stream, &mut data)?;

                header.set_entry_type(EntryType::Regular);
                header.set_size(data.len() as u64);
                header.set_cksum();

                archive_builder
                    .append_data(&mut header, path, data.as_slice())
                    .await
                    .map_err(Fix::Io)?;
            } else if file_type == InternalFileType::Symlink {
                let link = fs.read_link(path)?;
                let empty: &[u8] = &[];

                header.set_entry_type(EntryType::Symlink);
                header.set_link_name(link.to_str().unwrap())?;
                header.set_size(empty.len() as u64);
                header.set_cksum();

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
