use std::path::{Path, PathBuf};
use std::sync::Arc;

use eyre::Result;
use floppy_disk::prelude::*;
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

#[repr(transparent)]
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

    pub async fn size(&self) -> Result<u64> {
        let paths = traverse_memfs(self, Path::new("/"), Some(false)).await?;
        let mut size = 0u64;

        for path in paths {
            let metadata = self.fs.metadata(&path).await?;
            size += metadata.len();
        }

        Ok(size)
    }

    pub async fn resolve_symlink(&self, path: &Path) -> Result<PathBuf> {
        self.do_resolve_symlink(path, 0).await
    }

    #[async_recursion::async_recursion]
    async fn do_resolve_symlink(&self, path: &Path, depth: u8) -> Result<PathBuf> {
        if depth > 8 {
            return Err(eyre::eyre!(
                "too many symlinks (depth > 8), last path: {path:?}"
            ));
        }

        if path.is_symlink() {
            let link = self.fs.read_link(path).await?;
            self.do_resolve_symlink(&link, depth + 1).await
        } else {
            Ok(path.to_path_buf())
        }
    }
}

impl std::ops::Deref for MemFS {
    type Target = MemFloppyDisk;

    fn deref(&self) -> &MemFloppyDisk {
        &self.fs
    }
}

impl AsRef<MemFloppyDisk> for MemFS {
    fn as_ref(&self) -> &MemFloppyDisk {
        &self.fs
    }
}
