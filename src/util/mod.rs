use std::path::{Path, PathBuf};

use eyre::Result;
use log::*;
use rsfs_tokio::{DirEntry, GenFS, Metadata};
use thiserror::Error;
use tokio_stream::StreamExt;

use crate::fs::MemFS;

pub mod config;

#[derive(Error, Debug)]
pub enum Fix {
    #[error(transparent)]
    Anyhow(#[from] anyhow::Error),

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

#[async_recursion::async_recursion]
pub async fn traverse_memfs(
    memfs: &MemFS,
    root_path: &Path,
    push_directory_entries: Option<bool>,
) -> Result<Vec<PathBuf>> {
    let fs = memfs.as_ref();
    let mut paths = Vec::new();
    debug!("traversing memfs from {root_path:?}");

    let mut read_dir = fs.read_dir(root_path).await?;
    while let Some(entry) = read_dir.next().await {
        if let Some(entry) = entry? {
            let metadata = entry.metadata().await?;

            #[allow(clippy::if_same_then_else)]
            if metadata.is_dir() {
                let mut sub_paths =
                    traverse_memfs(memfs, &entry.path(), push_directory_entries).await?;
                if let Some(true) = push_directory_entries {
                    paths.push(entry.path());
                }
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
    // std::env::set_var("RUST_LOG", "DEBUG");
    std::env::set_var("RUST_BACKTRACE", "full");
    std::panic::catch_unwind(|| {
        if color_eyre::install().is_ok() {}
        pretty_env_logger::init();
    });
}
