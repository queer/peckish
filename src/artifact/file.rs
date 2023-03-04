use std::collections::HashMap;
use std::path::PathBuf;

use color_eyre::Result;
use rsfs::unix_ext::GenFSExt;
use rsfs::GenFS;
use tokio::fs::read_link;

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

        copy_files_from_paths_to_memfs(
            &self.paths.iter().map(|p| (p.clone(), p.clone())).collect(),
            &fs,
        )
        .await?;

        Ok(fs)
    }
}

/// Copies files from the host filesystem to a memory filesystem
/// Takes in a mapping of host paths -> memfs paths and a memfs.
pub async fn copy_files_from_paths_to_memfs(
    paths: &HashMap<PathBuf, PathBuf>,
    fs: &MemoryFS,
) -> Result<()> {
    for (path, memfs_path) in paths {
        let file_type = path.metadata()?.file_type();
        if file_type.is_dir() {
            fs.create_dir_all(path)?;
        } else if file_type.is_file() {
            let mut file_handle = fs.create_file(memfs_path)?;
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
            fs.symlink(path, link)?;
        } else {
            panic!("unknown file type for path {path:?}");
        }
    }

    Ok(())
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
            let mut file_handle = fs.open_file(path)?;

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
