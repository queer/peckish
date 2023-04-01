use std::os::unix::prelude::PermissionsExt;
use std::path::PathBuf;

use color_eyre::Result;
use log::*;
use rsfs_tokio::GenFS;

use crate::fs::{InternalFileType, MemFS};
use crate::util::config::Injection;
use crate::util::{is_in_tmp_dir, traverse_memfs, Fix};

use super::{Artifact, ArtifactProducer};

/// A path or set of paths on the filesystem.
#[derive(Debug, Clone)]
pub struct FileArtifact {
    pub name: String,
    pub paths: Vec<PathBuf>,
    /// Whether or not the contents of a path should be stripped of itself as a
    /// prefix. For example:
    ///
    /// ```text
    /// /a/b/c -> /c
    /// /a/b/c/d/e/... -> /...
    /// ```
    pub strip_path_prefixes: Option<bool>,
}

#[async_trait::async_trait]
impl Artifact for FileArtifact {
    fn name(&self) -> &str {
        &self.name
    }

    async fn extract(&self) -> Result<MemFS> {
        let fs = MemFS::new();

        debug!("copying {} paths to memfs!", self.paths.len());

        if let Some(true) = self.strip_path_prefixes {
            for path in &self.paths {
                let prefix = if path.is_dir() {
                    path
                } else if let Some(parent) = path.parent() {
                    parent
                } else {
                    path
                };
                fs.copy_files_from_paths(&vec![path.clone()], Some(prefix.into()))
                    .await?;
            }
        } else {
            fs.copy_files_from_paths(&self.paths, None).await?;
        }

        Ok(fs)
    }
}

/// Produces a set of files at the given path on the filesystem.
#[derive(Debug, Clone)]
pub struct FileProducer {
    pub name: String,
    pub path: PathBuf,
    pub injections: Vec<Injection>,
}

#[async_trait::async_trait]
impl ArtifactProducer for FileProducer {
    type Output = FileArtifact;

    fn name(&self) -> &str {
        &self.name
    }

    fn injections(&self) -> &[Injection] {
        &self.injections
    }

    async fn produce(&self, previous: &dyn Artifact) -> Result<FileArtifact> {
        let memfs = previous.extract().await?;
        let memfs = self.inject(&memfs).await?;
        let paths = traverse_memfs(memfs, &PathBuf::from("/")).await?;
        debug!("traversed memfs, found {} paths", paths.len());

        if let Some(parent) = self.path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        for path in &paths {
            use rsfs_tokio::unix_ext::PermissionsExt;
            use rsfs_tokio::{File, Metadata};

            debug!("processing path: {path:?}");
            let mut full_path = PathBuf::from("/");
            full_path.push(&self.path);
            full_path.push(path.strip_prefix("/")?);
            // If the path isn't in a tmp dir, or if the user didn't explicitly
            // specify that paths should end up at the root, strip the leading
            // `/` to avoid writing to the wrong place.
            let full_path = if is_in_tmp_dir(path)? || self.path.starts_with("/") {
                full_path
            } else {
                full_path.strip_prefix("/")?.to_path_buf()
            };
            debug!("full_path = {full_path:?}");
            if let Some(parent) = full_path.parent() {
                tokio::fs::create_dir_all(parent).await.map_err(Fix::Io)?;
            }

            let file_type = memfs.determine_file_type(path).await?;
            debug!("{path:?} is {file_type:?}");

            let fs = memfs.as_ref();
            if file_type == InternalFileType::File {
                debug!("writing file to {full_path:?}");
                let mut file = tokio::fs::File::create(&full_path).await?;
                let mut file_handle = fs.open_file(path).await?;
                tokio::io::copy(&mut file_handle, &mut file).await?;

                // Set permissions
                file.set_permissions(std::fs::Permissions::from_mode(
                    file_handle.metadata().await?.permissions().mode(),
                ))
                .await?;

                // Set ownership
                let metadata = file_handle.metadata().await?;
                let uid = metadata.uid()?;
                let gid = metadata.gid()?;

                nix::unistd::chown(
                    full_path.to_str().unwrap(),
                    Some(nix::unistd::Uid::from_raw(uid)),
                    Some(nix::unistd::Gid::from_raw(gid)),
                )?;
            } else if file_type == InternalFileType::Dir {
                debug!("creating dir {full_path:?}");
                tokio::fs::create_dir_all(&full_path)
                    .await
                    .map_err(Fix::Io)?;

                // Set permissions
                let metadata = fs.metadata(path).await?;
                let permissions = metadata.permissions();
                tokio::fs::set_permissions(
                    &full_path,
                    std::fs::Permissions::from_mode(permissions.mode()),
                )
                .await?;

                // Set ownership
                let uid = metadata.uid()?;
                let gid = metadata.gid()?;
                nix::unistd::chown(
                    full_path.to_str().unwrap(),
                    Some(nix::unistd::Uid::from_raw(uid)),
                    Some(nix::unistd::Gid::from_raw(gid)),
                )?;
            } else if file_type == InternalFileType::Symlink {
                let symlink_target = fs.read_link(path).await?;
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
            strip_path_prefixes: Some(true),
        })
    }
}
