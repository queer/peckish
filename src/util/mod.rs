use std::path::{Path, PathBuf};

use color_eyre::Result;
use log::*;
use rsfs_tokio::{DirEntry, GenFS, Metadata};
use thiserror::Error;
use tokio_stream::StreamExt;

pub mod config;

pub type MemoryFS = rsfs_tokio::mem::unix::FS;

#[derive(Error, Debug)]
pub enum Fix {
    #[error(transparent)]
    Anyhow(#[from] anyhow::Error),

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

#[async_recursion::async_recursion]
pub async fn traverse_memfs(fs: &MemoryFS, root_path: &Path) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    debug!("traversing memfs from {root_path:?}");

    let mut read_dir = fs.read_dir(root_path).await?;
    while let Some(entry) = read_dir.next().await {
        if let Some(entry) = entry? {
            let metadata = entry.metadata().await?;

            #[allow(clippy::if_same_then_else)]
            if metadata.is_dir() {
                let mut sub_paths = traverse_memfs(fs, &entry.path()).await?;
                paths.append(&mut sub_paths);
            } else if metadata.is_file() {
                paths.push(entry.path());
            } else if fs.read_link(entry.path()).await.is_ok() {
                paths.push(entry.path());
            }
        }
    }

    Ok(paths)
}

pub fn is_in_tmp_dir(path: &Path) -> Result<bool> {
    Ok(path.starts_with("/tmp/peckish-"))
}

#[cfg(test)]
#[allow(unused_must_use)]
pub fn test_init() {
    std::env::set_var("RUST_LOG", "DEBUG");
    std::env::set_var("RUST_BACKTRACE", "full");
    std::panic::catch_unwind(|| {
        // TODO: This logs a crash but it works
        color_eyre::install().unwrap();
        pretty_env_logger::init();
    });
}
