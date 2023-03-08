use std::path::{Path, PathBuf};

use color_eyre::Result;
use log::*;

use crate::util::Fix;

#[derive(Clone)]
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
