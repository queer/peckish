use std::path::{Path, PathBuf};

use eyre::{eyre, Result};
use floppy_disk::{FloppyDirEntry, FloppyDisk, FloppyMetadata, FloppyReadDir};
use thiserror::Error;
use tracing::*;

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
    while let Some(entry) = read_dir.next_entry().await? {
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

    Ok(paths)
}

#[cfg(test)]
#[allow(unused_must_use)]
pub fn test_init() {
    // std::env::set_var("RUST_LOG", "DEBUG");
    std::env::set_var("RUST_BACKTRACE", "full");
    std::panic::catch_unwind(|| {
        if color_eyre::install().is_ok() {}
        let subscriber = tracing_subscriber::FmtSubscriber::builder()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .finish();
        tracing::subscriber::set_global_default(subscriber)
            .expect("setting default subscriber failed");
    });
}

pub fn get_current_time() -> Result<u64> {
    if let Ok(source_date_epoch) = std::env::var("SOURCE_DATE_EPOCH") {
        let source_date_epoch = source_date_epoch.parse::<u64>()?;
        let current_time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs();
        if source_date_epoch > current_time {
            return Err(eyre!("SOURCE_DATE_EPOCH is set to a time in the future"));
        }
        Ok(source_date_epoch)
    } else {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs();
        Ok(now)
    }
}
