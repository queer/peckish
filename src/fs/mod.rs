use std::os::unix::prelude::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use eyre::eyre;
use eyre::Result;
use floppy_disk::mem::{MemOpenOptions, MemPermissions};
use floppy_disk::prelude::*;
use tokio::fs::read_link;
use tracing::*;

use crate::util::{traverse_memfs, Fix};

pub struct TempDir {
    path: PathBuf,
}

impl TempDir {
    pub async fn new() -> Result<TempDir> {
        let mut path = std::env::temp_dir();
        path.push(format!("peckish-workdir-{}", rand::random::<u64>()));
        tokio::fs::create_dir_all(&path).await.map_err(Fix::Io)?;

        Ok(TempDir { path })
    }

    pub fn path_view(&self) -> PathBuf {
        self.path.clone()
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        debug!("!!! DROPPING TEMP DIR {:?}", self.path);
        if self.path.exists() {
            std::fs::remove_dir_all(&self.path).unwrap();
        }
    }
}

impl AsRef<Path> for TempDir {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

impl AsRef<PathBuf> for TempDir {
    fn as_ref(&self) -> &PathBuf {
        &self.path
    }
}

impl std::ops::Deref for TempDir {
    type Target = PathBuf;

    fn deref(&self) -> &Self::Target {
        &self.path
    }
}

/// A `MemFS` is a memory-backed filesystem. It is a wrapper around
/// `rsfs_tokio` that helps with manipulation of things like temporary paths
/// that would otherwise be difficult to know about.
#[derive(Debug, Clone)]
pub struct MemFS {
    fs: Arc<MemFloppyDisk>,
}

impl MemFS {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        MemFS {
            fs: Arc::new(MemFloppyDisk::new()),
        }
    }

    /// Copies files from the host filesystem to a memory filesystem
    /// Takes in a mapping of host paths -> memfs paths and a memfs.
    /// Allows an optional prefix to strip so that ex. temporary workdirs can
    /// be used as expected.
    ///
    /// * `paths`: A list of paths to copy from the host filesystem to the
    ///            memory filesystem.
    /// * `view_of`: An optional path that the paths are relative to. If
    ///              provided, the paths will be copied to the memory
    ///              filesystem with the view path stripped from the beginning.
    // TODO: What about xattrs?
    pub async fn copy_files_from_paths(
        &mut self,
        paths: &Vec<PathBuf>,
        view_of: Option<PathBuf>,
        dest: Option<PathBuf>,
    ) -> Result<()> {
        for path in paths {
            let memfs_path = if let Some(ref view) = view_of {
                let path = path.strip_prefix(view)?;

                // If the path is an empty string, replace it with just "/"
                if path == Path::new("") {
                    Path::new("/")
                } else {
                    path
                }
            } else {
                path
            };
            let memfs_path = if let Some(ref dest) = dest {
                dest.join(memfs_path)
            } else {
                memfs_path.to_path_buf()
            };
            debug!("{} -> {}", path.display(), memfs_path.display());
            let file_type = self.determine_file_type_from_filesystem(path).await?;
            if file_type == InternalFileType::Dir {
                self.copy_dir_to_memfs(path, &memfs_path).await?;
            } else if file_type == InternalFileType::File {
                self.copy_file_to_memfs(path, &memfs_path).await?;
            } else if file_type == InternalFileType::Symlink {
                self.add_symlink_to_memfs(path, &memfs_path).await?;
            } else {
                error!("unknown file type for path {path:?}");
            }
        }

        Ok(())
    }

    pub(crate) async fn copy_file_to_memfs(
        &mut self,
        path: &Path,
        memfs_path: &Path,
    ) -> Result<()> {
        debug!("creating file {path:?}");
        if let Some(memfs_parent) = memfs_path.parent() {
            self.fs.create_dir_all(memfs_parent).await?;
        }

        let mut file_handle = MemOpenOptions::new()
            .create(true)
            .write(true)
            .open(&self.fs, memfs_path)
            .await?;
        let path_clone = path.to_path_buf();
        let mut file = tokio::fs::File::open(path_clone).await?;
        tokio::io::copy(&mut file, &mut file_handle)
            .await
            .map_err(Fix::Io)?;

        let mode = file.metadata().await?.permissions().mode();
        let permissions = MemPermissions::from_mode(mode);
        self.fs.set_permissions(memfs_path, permissions).await?;

        let host_file = tokio::fs::metadata(path).await?;
        let uid = host_file.uid();
        let gid = host_file.gid();
        self.fs.chown(memfs_path, uid, gid).await?;

        Ok(())
    }

    #[async_recursion::async_recursion]
    pub(crate) async fn copy_dir_to_memfs(&mut self, path: &Path, memfs_path: &Path) -> Result<()> {
        self.fs.create_dir_all(memfs_path).await?;

        let host_dir = tokio::fs::metadata(&path).await?;
        let mode = host_dir.permissions().mode();
        let permissions = MemPermissions::from_mode(mode);
        self.fs.set_permissions(memfs_path, permissions).await?;
        self.fs
            .chown(memfs_path, host_dir.uid(), host_dir.gid())
            .await?;

        let mut files = tokio::fs::read_dir(path).await?;

        while let Some(file) = files.next_entry().await? {
            let file_type = file.file_type().await?;
            let mut file_path = memfs_path.to_path_buf();
            file_path.push(file.file_name());
            if file_type.is_dir() {
                self.copy_dir_to_memfs(&file.path(), &file_path).await?;
            } else if file_type.is_file() {
                self.copy_file_to_memfs(&file.path(), &file_path).await?;
            } else if file_type.is_symlink() {
                self.add_symlink_to_memfs(&file.path(), &file_path).await?;
            }
        }

        Ok(())
    }

    pub(crate) async fn add_symlink_to_memfs(&self, path: &Path, memfs_path: &Path) -> Result<()> {
        let link = read_link(&path).await.map_err(Fix::Io)?;
        debug!("linking {memfs_path:?} to {link:?}");
        self.fs.symlink(link, memfs_path.into()).await?;

        Ok(())
    }

    pub async fn determine_file_type(&self, path: &Path) -> Result<InternalFileType> {
        match self.fs.read_link(path).await {
            Ok(_) => Ok(InternalFileType::Symlink),
            Err(_) => {
                let file_type = self.fs.metadata(path).await?.file_type();
                if file_type.is_symlink() {
                    Ok(InternalFileType::Symlink)
                } else if file_type.is_dir() {
                    Ok(InternalFileType::Dir)
                } else if file_type.is_file() {
                    Ok(InternalFileType::File)
                } else {
                    Err(eyre!("unknown file type {file_type:?} for path {path:?}"))
                }
            }
        }
    }

    pub async fn determine_file_type_from_filesystem(
        &self,
        path: &Path,
    ) -> Result<InternalFileType> {
        debug!("determining type of {path:?}");
        match tokio::fs::read_link(path).await {
            Ok(_) => Ok(InternalFileType::Symlink),
            Err(_) => {
                let file_type = tokio::fs::metadata(path).await?.file_type();
                if file_type.is_dir() {
                    Ok(InternalFileType::Dir)
                } else if file_type.is_file() {
                    Ok(InternalFileType::File)
                } else if file_type.is_symlink() {
                    Ok(InternalFileType::Symlink)
                } else {
                    Err(eyre!("unknown file type {file_type:?} for path {path:?}"))
                }
            }
        }
    }

    pub async fn resolve_symlink(&self, path: &Path) -> Result<PathBuf> {
        self.do_resolve_symlink(path, 0).await
    }

    #[async_recursion::async_recursion]
    async fn do_resolve_symlink(&self, path: &Path, depth: u8) -> Result<PathBuf> {
        if depth > 8 {
            return Err(eyre!("too many symlinks (depth > 8), last path: {path:?}"));
        }

        if path.is_symlink() {
            let link = self.fs.read_link(path).await?;
            self.do_resolve_symlink(&link, depth + 1).await
        } else {
            Ok(path.to_path_buf())
        }
    }

    pub async fn size(&self) -> Result<u64> {
        let paths = traverse_memfs(self, Path::new("/"), Some(false)).await?;
        let mut size = 0u64;

        for path in paths {
            let metadata = self.fs.metadata(&path).await?;
            size += metadata.len();
        }

        Ok(size)
    }
}

impl AsRef<MemFloppyDisk> for MemFS {
    fn as_ref(&self) -> &MemFloppyDisk {
        &self.fs
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum InternalFileType {
    Dir,
    File,
    Symlink,
}
