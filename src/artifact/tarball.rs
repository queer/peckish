use std::path::PathBuf;

use color_eyre::Result;
use rsfs::GenFS;
use tokio::fs::File;
use tokio_stream::StreamExt;
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

        let mut archive = Archive::new(File::open(&self.path).await.map_err(Fix::Io)?);
        let mut entries = archive.entries().map_err(Fix::Io)?;

        while let Some(file) = entries.next().await {
            let file = file.map_err(Fix::Io)?;
            let path = file.path().map_err(Fix::Io)?;
            let mut directory_create_buf = PathBuf::from("/");
            let is_dir = file.header().entry_type() == EntryType::Directory;

            for component in path.components() {
                directory_create_buf.push(component);
                if !is_dir && component == path.components().last().unwrap() {
                    break;
                } else {
                    fs.create_dir(directory_create_buf.to_string_lossy().as_ref())?;
                }
            }

            if !is_dir {
                let mut file_handle = fs.create_file(path.to_string_lossy().as_ref())?;

                let mut file = std::fs::File::open(path).map_err(Fix::Io)?;
                std::io::copy(&mut file, &mut file_handle).map_err(Fix::Io)?;
            }
        }

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
        println!("read paths from vfs: {paths:?}");

        let file = File::create(&self.out).await.map_err(Fix::Io)?;
        let mut archive_builder = tokio_tar::Builder::new(file);
        for path in paths {
            let mut stream = fs.open_file(path.to_string_lossy().as_ref())?;
            let path = path.strip_prefix("/")?;

            let mut data = Vec::new();
            std::io::copy(&mut stream, &mut data)?;

            let mut header = Header::new_gnu();
            header
                .set_path(path)
                .map_err(Fix::Io)?;

            header.set_size(data.len() as u64);

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
