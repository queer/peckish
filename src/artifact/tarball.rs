use std::path::PathBuf;

use color_eyre::Result;
use rsfs::unix_ext::GenFSExt;
use rsfs::{FileType, GenFS, Metadata};
use tokio::fs::{read_link, File};
use tokio_tar::{Archive, EntryType, Header};

use crate::util::{traverse_memfs, Fix, MemoryFS};

use super::{Artifact, ArtifactProducer};

pub struct TarballArtifact {
    pub name: String,
    pub path: PathBuf,
}

#[async_trait::async_trait]
impl Artifact for TarballArtifact {
    fn name(&self) -> &String {
        &self.name
    }

    fn description(&self) -> String {
        "An artifact of one or more files".to_string()
    }

    async fn extract(&self) -> Result<MemoryFS> {
        let fs = MemoryFS::new();

        // Unpack TAR to a temporary archive, then copy it to the memory filesystem
        let mut archive = Archive::new(File::open(&self.path).await.map_err(Fix::Io)?);
        let mut tmp = std::env::temp_dir();
        tmp.push(format!("peckish_unpack-{}", rand::random::<u64>()));
        archive.unpack(&tmp).await.map_err(Fix::Io)?;
        let walk_results = nyoom::walk(&tmp, |_path, _| ())?;
        for e in walk_results.paths.iter() {
            let path = e.key().clone();
            let file_type = path.metadata()?.file_type();
            let path = path.strip_prefix(&tmp)?.to_path_buf();

            // TODO: DRY with FileArtifact
            if file_type.is_dir() {
                fs.create_dir_all(path)?;
            } else if file_type.is_file() {
                let mut file_handle = fs.create_file(&path)?;
                let path_clone = path.clone();
                let join_handle = tokio::spawn(async move {
                    let mut file = std::fs::File::open(path_clone).map_err(Fix::Io).unwrap();
                    std::io::copy(&mut file, &mut file_handle)
                        .map_err(Fix::Io)
                        .unwrap();
                });
                join_handle.await?;
            } else if file_type.is_symlink() {
                let link = read_link(&path).await.map_err(Fix::Io)?;
                fs.symlink(&path, link)?;
            } else {
                panic!("unknown file type for path {path:?}");
            }
        }
        tokio::fs::remove_dir_all(tmp).await.map_err(Fix::Io)?;

        Ok(fs)
    }
}

pub struct TarballProducer {
    pub out: String,
}

#[async_trait::async_trait]
impl ArtifactProducer<TarballArtifact> for TarballProducer {
    async fn produce(&self, previous: &dyn Artifact) -> Result<TarballArtifact> {
        let fs = previous.extract().await?;
        let paths = traverse_memfs(&fs, &PathBuf::from("/"))?;

        let file = File::create(&self.out).await.map_err(Fix::Io)?;
        let mut archive_builder = tokio_tar::Builder::new(file);
        for path in paths {
            let mut stream = fs.open_file(&path)?;
            let path = path.strip_prefix("/")?;

            let mut data = Vec::new();
            std::io::copy(&mut stream, &mut data)?;

            let mut header = Header::new_gnu();
            header.set_path(path).map_err(Fix::Io)?;

            let file_type = fs.metadata(path)?.file_type();
            if file_type.is_dir() {
                header.set_entry_type(EntryType::Directory);
                header.set_size(0);
            } else if file_type.is_file() {
                header.set_size(data.len() as u64);
            } else if file_type.is_symlink() {
            }

            archive_builder
                .append_data(&mut header, path, data.as_slice())
                .await
                .map_err(Fix::Io)?;
        }

        Ok(TarballArtifact {
            name: self.out.clone(),
            path: self.out.clone().into(),
        })
    }
}
