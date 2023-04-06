use std::os::unix::prelude::PermissionsExt;
use std::path::PathBuf;

use eyre::Result;
use log::*;
use rsfs_tokio::GenFS;

use crate::fs::{InternalFileType, MemFS};
use crate::util::config::Injection;
use crate::util::{is_in_tmp_dir, traverse_memfs, Fix};

use super::{Artifact, ArtifactProducer, SelfBuilder, SelfValidation};

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

    fn try_clone(&self) -> Result<Box<dyn Artifact>> {
        Ok(Box::new(self.clone()))
    }
}

#[async_trait::async_trait]
impl SelfValidation for FileArtifact {
    async fn validate(&self) -> Result<()> {
        let mut errors = vec![];

        for path in &self.paths {
            if !path.exists() {
                errors.push(format!("path does not exist: {path:?}"));
            } else if !path.is_file() && !path.is_dir() {
                errors.push(format!("path is not a file or directory: {path:?}"));
            }
        }

        if !errors.is_empty() {
            return Err(eyre::eyre!(
                "File artifact not valid:\n{}",
                errors.join("\n")
            ));
        }

        Ok(())
    }
}

pub struct FileArtifactBuilder {
    name: String,
    paths: Vec<PathBuf>,
    strip_path_prefixes: Option<bool>,
    preserve_empty_directories: Option<bool>,
}

#[allow(unused)]
impl FileArtifactBuilder {
    pub fn add_path<P: Into<PathBuf>>(&mut self, path: P) -> &mut Self {
        self.paths.push(path.into());
        self
    }

    pub fn strip_path_prefixes(&mut self, strip: bool) -> &mut Self {
        self.strip_path_prefixes = Some(strip);
        self
    }

    pub fn preserve_empty_directories(&mut self, preserve: bool) -> &mut Self {
        self.preserve_empty_directories = Some(preserve);
        self
    }
}

impl SelfBuilder for FileArtifactBuilder {
    type Output = FileArtifact;

    fn new<S: Into<String>>(name: S) -> Self {
        Self {
            name: name.into(),
            paths: vec![],
            strip_path_prefixes: None,
            preserve_empty_directories: None,
        }
    }

    fn build(&self) -> Result<Self::Output> {
        Ok(FileArtifact {
            name: self.name.clone(),
            paths: self.paths.clone(),
            strip_path_prefixes: self.strip_path_prefixes,
        })
    }
}

/// Produces a set of files at the given path on the filesystem.
#[derive(Debug, Clone)]
pub struct FileProducer {
    pub name: String,
    pub path: PathBuf,
    pub preserve_empty_directories: Option<bool>,
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

    async fn produce_from(&self, previous: &dyn Artifact) -> Result<FileArtifact> {
        let memfs = previous.extract().await?;
        debug!("injecting memfs");
        let memfs = self.inject(&memfs).await?;
        debug!("traversing memfs");
        let paths =
            traverse_memfs(memfs, &PathBuf::from("/"), self.preserve_empty_directories).await?;
        debug!("traversed memfs, found {} paths", paths.len());

        if let Some(parent) = self.path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let mut output_paths = vec![];
        for path in &paths {
            use rsfs_tokio::unix_ext::PermissionsExt;
            use rsfs_tokio::{File, Metadata};

            debug!("processing path: {path:?} -> {:?}", self.path);
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
            if is_in_tmp_dir(path)? || self.path.starts_with("/") {
                output_paths.push(full_path.to_path_buf());
            } else {
                output_paths.push(
                    full_path
                        .strip_prefix("/")
                        .unwrap_or(&full_path)
                        .to_path_buf(),
                );
            };
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

                debug!("chown {full_path:?} to {uid}:{gid}");
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
            } else if file_type == InternalFileType::Symlink {
                let symlink_target = fs.read_link(path).await?;
                debug!("creating symlink {full_path:?} -> {symlink_target:?}");
                tokio::fs::symlink(symlink_target, full_path)
                    .await
                    .map_err(Fix::Io)?;
            }
        }

        debug!("collected {} paths", output_paths.len());

        Ok(FileArtifact {
            name: self.path.to_string_lossy().to_string(),
            paths: output_paths,
            strip_path_prefixes: Some(true),
        })
    }
}

#[async_trait::async_trait]
impl SelfValidation for FileProducer {
    async fn validate(&self) -> Result<()> {
        tokio::fs::create_dir_all(&self.path).await?;

        Ok(())
    }
}

pub struct FileProducerBuilder {
    name: String,
    path: PathBuf,
    preserve_empty_directories: Option<bool>,
    injections: Vec<Injection>,
}

#[allow(unused)]
impl FileProducerBuilder {
    pub fn path<P: Into<PathBuf>>(mut self, path: P) -> Self {
        self.path = path.into();
        self
    }

    pub fn preserve_empty_directories(mut self, preserve_empty_directories: bool) -> Self {
        self.preserve_empty_directories = Some(preserve_empty_directories);
        self
    }

    pub fn inject(mut self, injection: Injection) -> Self {
        self.injections.push(injection);
        self
    }
}

impl SelfBuilder for FileProducerBuilder {
    type Output = FileProducer;

    fn new<S: Into<String>>(name: S) -> Self {
        Self {
            name: name.into(),
            path: PathBuf::from("/"),
            preserve_empty_directories: None,
            injections: vec![],
        }
    }

    fn build(&self) -> Result<Self::Output> {
        Ok(FileProducer {
            name: self.name.clone(),
            path: self.path.clone(),
            preserve_empty_directories: self.preserve_empty_directories,
            injections: self.injections.clone(),
        })
    }
}
