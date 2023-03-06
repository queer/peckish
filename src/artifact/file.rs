use std::path::PathBuf;

use color_eyre::Result;
use log::*;
use rsfs::GenFS;

use crate::util::config::Injection;
use crate::util::{traverse_memfs, Fix, MemoryFS};

use super::{copy_files_from_paths_to_memfs, Artifact, ArtifactProducer, InternalFileType};

#[derive(Debug, Clone)]
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

        debug!("copying {} paths to memfs!", self.paths.len());

        copy_files_from_paths_to_memfs(
            &self.paths.iter().map(|p| (p.clone(), p.clone())).collect(),
            &fs,
        )
        .await?;

        Ok(fs)
    }
}

#[derive(Debug, Clone)]
pub struct FileProducer {
    pub name: String,
    pub path: PathBuf,
    pub injections: Vec<Injection>,
}

#[async_trait::async_trait]
impl ArtifactProducer for FileProducer {
    type Output = FileArtifact;

    fn name(&self) -> &String {
        &self.name
    }

    fn injections(&self) -> &[Injection] {
        &self.injections
    }

    async fn produce(&self, previous: &dyn Artifact) -> Result<FileArtifact> {
        let fs = previous.extract().await?;
        let fs = self.inject(&fs)?;
        let paths = traverse_memfs(fs, &PathBuf::from("/"))?;
        debug!("traversed memfs, found {} paths", paths.len());

        for path in &paths {
            debug!("processing path: {path:?}");
            let mut full_path = PathBuf::from("/");
            full_path.push(&self.path);
            full_path.push(path.strip_prefix("/")?);
            let full_path = full_path.strip_prefix("/")?;
            debug!("full_path = {full_path:?}");
            if let Some(parent) = full_path.parent() {
                tokio::fs::create_dir_all(parent).await.map_err(Fix::Io)?;
            }

            let file_type = super::determine_file_type_from_memfs(fs, path)?;
            debug!("{path:?} is {file_type:?}");

            if file_type == InternalFileType::File {
                debug!("writing file to {full_path:?}");
                let mut file = std::fs::File::create(full_path).map_err(Fix::Io).unwrap();
                let mut file_handle = fs.open_file(path).unwrap();
                let join_handle = tokio::spawn(async move {
                    std::io::copy(&mut file_handle, &mut file)
                        .map_err(Fix::Io)
                        .unwrap();
                });
                join_handle.await?;
            } else if file_type == InternalFileType::Dir {
                debug!("creating dir {full_path:?}");
                tokio::fs::create_dir_all(full_path)
                    .await
                    .map_err(Fix::Io)?;
            } else if file_type == InternalFileType::Symlink {
                let symlink_target = fs.read_link(path)?;
                debug!("creating symlink {full_path:?} -> {symlink_target:?}");
                tokio::fs::symlink(symlink_target, full_path)
                    .await
                    .map_err(Fix::Io)?;
            }
        }

        let paths: Vec<PathBuf> = paths
            .iter()
            .map(|p| p.strip_prefix("/").unwrap().to_path_buf())
            .collect();

        debug!("collected {} paths", paths.len());

        Ok(FileArtifact {
            name: self.path.to_string_lossy().to_string(),
            paths,
        })
    }
}
