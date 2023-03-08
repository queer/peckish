use std::collections::HashMap;
use std::path::{Path, PathBuf};

use color_eyre::eyre::eyre;
use color_eyre::Result;
use log::*;
use rsfs_tokio::unix_ext::GenFSExt;
use rsfs_tokio::{FileType, GenFS, Metadata};
use tokio::fs::read_link;

use crate::util::config::Injection;
use crate::util::{traverse_memfs, Fix, MemoryFS};

pub mod arch;
pub mod docker;
pub mod file;
pub mod tarball;

/// An artifact is the result of some build process.
#[async_trait::async_trait]
pub trait Artifact: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;

    /// Extract this artifact into a virtual filesystem. Used for manipulating
    /// the artifact's contents.
    async fn extract(&self) -> Result<MemoryFS>;
}

/// An artifact producer takes in the previous artifact and produces a new one.
#[async_trait::async_trait]
pub trait ArtifactProducer {
    type Output: Artifact;

    fn name(&self) -> &str;

    fn injections(&self) -> &[Injection];

    /// Produce a new artifact, given a previous artifact.
    async fn produce(&self, previous: &dyn Artifact) -> Result<Self::Output>;

    /// Inject this producer's changes into the memfs.
    async fn inject<'a>(&self, fs: &'a MemoryFS) -> Result<&'a MemoryFS> {
        for injection in self.injections() {
            debug!("applying injection {injection:?}");
            injection.inject(fs).await?;
        }

        Ok(fs)
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum InternalFileType {
    Dir,
    File,
    Symlink,
}

/// Copies files from the host filesystem to a memory filesystem
/// Takes in a mapping of host paths -> memfs paths and a memfs.
/// Allows an optional prefix to strip so that ex. temporary workdirs can be
/// used as expected.
// TODO: Preserve timestamps
// TODO: Preserve permissions
// TODO: Preserve ownership
// TODO: Other metadata?
pub async fn copy_files_from_paths_to_memfs(
    paths: &HashMap<PathBuf, PathBuf>,
    fs: &MemoryFS,
) -> Result<()> {
    for (path, memfs_path) in paths {
        let file_type = determine_file_type_from_filesystem(path).await?;
        debug!("copying {path:?} ({file_type:?}) to {memfs_path:?}");
        if file_type == InternalFileType::Dir {
            copy_dir_to_memfs(path, memfs_path, fs).await?;
        } else if file_type == InternalFileType::File {
            copy_file_to_memfs(path, memfs_path, fs).await?;
        } else if file_type == InternalFileType::Symlink {
            add_symlink_to_memfs(path, memfs_path, fs).await?;
        } else {
            error!("unknown file type for path {path:?}");
        }
    }

    Ok(())
}

async fn copy_file_to_memfs(path: &Path, memfs_path: &Path, fs: &MemoryFS) -> Result<()> {
    debug!("creating file {path:?}");
    if let Some(memfs_parent) = memfs_path.parent() {
        fs.create_dir_all(memfs_parent).await?;
    }

    let mut file_handle = fs.create_file(memfs_path).await?;
    let path_clone = path.to_path_buf();
    let mut file = tokio::fs::File::open(path_clone).await?;
    tokio::io::copy(&mut file, &mut file_handle)
        .await
        .map_err(Fix::Io)?;

    Ok(())
}

#[async_recursion::async_recursion]
async fn copy_dir_to_memfs(path: &Path, memfs_path: &Path, fs: &MemoryFS) -> Result<()> {
    debug!("creating dir {memfs_path:?}");
    fs.create_dir_all(memfs_path).await?;

    let mut files = tokio::fs::read_dir(path).await?;

    while let Some(file) = files.next_entry().await? {
        let file_type = file.file_type().await?;
        let mut file_path = memfs_path.to_path_buf();
        file_path.push(file.file_name());
        if file_type.is_dir() {
            copy_dir_to_memfs(&file.path(), &file_path, fs).await?;
        } else if file_type.is_file() {
            copy_file_to_memfs(&file.path(), &file_path, fs).await?;
        } else if file_type.is_symlink() {
            add_symlink_to_memfs(&file.path(), &file_path, fs).await?;
        }
    }

    Ok(())
}

async fn add_symlink_to_memfs(path: &Path, memfs_path: &Path, fs: &MemoryFS) -> Result<()> {
    let link = read_link(&path).await.map_err(Fix::Io)?;
    debug!("linking {memfs_path:?} to {link:?}");
    fs.symlink(link, memfs_path).await?;

    Ok(())
}

/// The rsfs method doessn't handle symlinks right for some reason.
pub async fn determine_file_type_from_memfs(
    fs: &MemoryFS,
    path: &Path,
) -> Result<InternalFileType> {
    match fs.read_link(path).await {
        Ok(_) => Ok(InternalFileType::Symlink),
        Err(_) => {
            let file_type = fs.metadata(path).await?.file_type();
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

pub async fn determine_file_type_from_filesystem(path: &Path) -> Result<InternalFileType> {
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

pub async fn get_artifact_size(artifact: &dyn Artifact) -> Result<u64> {
    let fs = artifact.extract().await?;
    let paths = traverse_memfs(&fs, Path::new("/")).await?;
    let mut size = 0u64;

    for path in paths {
        let metadata = fs.metadata(&path).await?;
        size += metadata.len();
    }

    Ok(size)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use color_eyre::Result;

    use crate::util::Fix;

    use super::*;

    #[ctor::ctor]
    fn init() {
        crate::util::test_init();
    }

    #[tokio::test]
    async fn test_basic_transform_works() -> Result<()> {
        let file_artifact = file::FileArtifact {
            name: "Cargo.toml".into(),
            paths: vec![PathBuf::from("Cargo.toml")],
        };

        let tarball_producer = tarball::TarballProducer {
            name: "test-tarball-producer".into(),
            path: "test.tar.gz".into(),
            injections: vec![],
        };

        let tarball_artifact = tarball_producer.produce(&file_artifact).await?;

        assert_eq!(tarball_artifact.name(), "test.tar.gz");
        let tarball_path = PathBuf::from(tarball_artifact.name());
        assert!(tarball_path.exists());

        let file_producer = file::FileProducer {
            name: "test-file-producer".into(),
            path: "test".into(),
            injections: vec![],
        };

        let file_artifact = file_producer.produce(&tarball_artifact).await?;

        assert_eq!(file_artifact.name(), "test");
        for path in &file_artifact.paths {
            assert!(path.exists());
        }

        tokio::fs::remove_file(tarball_artifact.name())
            .await
            .map_err(Fix::Io)?;
        tokio::fs::remove_dir_all(file_artifact.name())
            .await
            .map_err(Fix::Io)?;

        Ok(())
    }
}
