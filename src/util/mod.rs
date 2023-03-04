use std::path::{Path, PathBuf};

use color_eyre::Result;
use rsfs::{DirEntry, GenFS, Metadata};
use thiserror::Error;

pub type MemoryFS = rsfs::mem::unix::FS;

#[derive(Error, Debug)]
pub enum Fix {
    #[error(transparent)]
    Anyhow(#[from] anyhow::Error),

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub fn traverse_memfs(fs: &MemoryFS, root_path: &Path) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();

    for path in fs.read_dir(root_path)? {
        let path = path?;
        if path.metadata()?.is_dir() {
            let mut sub_paths = traverse_memfs(fs, &path.path())?;
            paths.append(&mut sub_paths);
        } else {
            paths.push(path.path());
        }
    }

    Ok(paths)
}
