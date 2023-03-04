use std::path::PathBuf;

use color_eyre::Result;
use rsfs::GenFS;

use crate::util::{traverse_memfs, Fix, MemoryFS};

use super::{Artifact, ArtifactProducer};

pub struct FileArtifact {
    pub name: String,
    pub paths: Vec<PathBuf>,
}

#[async_trait::async_trait]
impl Artifact for FileArtifact {
    fn name(&self) -> &String {
        &self.name
    }

    fn description(&self) -> String {
        "An artifact of one or more files".to_string()
    }

    async fn extract(&self) -> Result<MemoryFS> {
        let fs = MemoryFS::new();

        for path in &self.paths {
            if path.is_dir() {
                fs.create_dir_all(path.to_string_lossy().as_ref())?;
            } else {
                let mut file_handle = fs.create_file(path.to_string_lossy().as_ref())?;
                let mut file = std::fs::File::open(path).map_err(Fix::Io)?;
                std::io::copy(&mut file, &mut file_handle).map_err(Fix::Io)?;
            }
        }

        Ok(fs)
    }
}

pub struct FileProducer {
    pub out: String,
}

#[async_trait::async_trait]
impl ArtifactProducer<FileArtifact> for FileProducer {
    async fn produce(&self, previous: &dyn Artifact) -> Result<FileArtifact> {
        let fs = previous.extract().await?;
        let paths = traverse_memfs(&fs, &PathBuf::from("/"))?;

        for path in &paths {
            let mut full_path = PathBuf::from("/");
            full_path.push(&self.out);
            full_path.push(path.strip_prefix("/")?);
            let full_path = full_path.strip_prefix("/")?;
            if let Some(parent) = full_path.parent() {
                std::fs::create_dir_all(parent).map_err(Fix::Io)?;
            }
            let mut file = std::fs::File::create(full_path).map_err(Fix::Io)?;
            let mut file_handle = fs.open_file(path.to_string_lossy().as_ref())?;

            std::io::copy(&mut file_handle, &mut file).map_err(Fix::Io)?;
        }

        let paths = paths
            .iter()
            .map(|p| p.strip_prefix("/").unwrap().to_path_buf())
            .collect();

        Ok(FileArtifact {
            name: self.out.clone(),
            paths,
        })
    }
}
