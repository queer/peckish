use std::path::{Path, PathBuf};
use std::sync::Arc;

use eyre::Result;
use floppy_disk::prelude::*;
use tracing::*;

use crate::util::Fix;

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
            // std::fs::remove_dir_all(&self.path).unwrap();
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
        let paths = nyoom::walk(self.fs.as_ref(), "/").await?;
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

#[cfg(test)]
pub(crate) mod test_utils {
    use std::path::PathBuf;

    use tracing::debug;

    use super::TempDir;

    pub struct Fixture {
        pub which: String,
        #[allow(dead_code)]
        temp_dir: TempDir,
    }

    impl Fixture {
        pub async fn new(which: &str) -> Fixture {
            let temp_dir = TempDir::new().await.unwrap();
            let path = Self::path(which);
            debug!("copying {:?} to {:?}", path, temp_dir.path_view());
            // create the file in the temp dir
            tokio::fs::copy(path, temp_dir.path_view().join(which))
                .await
                .unwrap();

            Fixture {
                which: which.to_string(),
                temp_dir,
            }
        }

        fn path(which: &str) -> PathBuf {
            let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            path.push("test");
            path.push("fixtures");
            path.push(which);
            path
        }

        pub fn path_view(&self) -> PathBuf {
            let mut path = self.temp_dir.path_view();
            path.push(&self.which);
            path
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            debug!("!!! DROPPING FIXTURE {:?}", self.which);
        }
    }
}
