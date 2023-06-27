use std::path::PathBuf;

use eyre::Result;
use tracing::*;

use crate::fs::MemFS;
use crate::util::config::Injection;

pub mod arch;
pub mod deb;
pub mod docker;
pub mod ext4;
pub mod file;
pub(crate) mod memory;
pub mod rpm;
pub mod tarball;

/// An artifact is (usually) the result of some build process.
#[async_trait::async_trait]
pub trait Artifact: Send + Sync + SelfValidation {
    fn name(&self) -> &str;

    /// Extract this artifact into a virtual filesystem. Used for manipulating
    /// the artifact's contents.
    async fn extract(&self) -> Result<MemFS>;

    /// We can't require `Clone` bounds because then it's not object-safe.
    fn try_clone(&self) -> Result<Box<dyn Artifact>>;

    /// A list of paths on the filesystem that this artifact represents. This
    /// value is optional, as ex. Docker artifacts don't "exist" on the
    /// filesystem, but are instead pulled from the daemon.
    fn paths(&self) -> Option<Vec<PathBuf>>;
}

/// An artifact producer takes in the previous artifact and produces a new one.
#[async_trait::async_trait]
pub trait ArtifactProducer: SelfValidation {
    type Output: Artifact;

    fn name(&self) -> &str;

    fn injections(&self) -> &[Injection];

    /// Produce a new artifact, given a previous artifact.
    async fn produce_from(&self, previous: &dyn Artifact) -> Result<Self::Output>;

    /// Inject this producer's custom changes into the memfs.
    async fn inject<'a>(&self, fs: &'a mut MemFS) -> Result<()> {
        for injection in self.injections() {
            debug!("applying injection {injection:?}");
            injection.inject(fs).await?;
        }

        Ok(())
    }
}

/// Self-validation for structs! Because all structs should feel good about
/// themselves :D
///
/// But seriously, this is to let [`Artifact`] and [`ArtifactProducer`] be able
/// to do some self-validation of the values they've been configured with,
/// before they just run wild.
#[async_trait::async_trait]
pub trait SelfValidation {
    async fn validate(&self) -> Result<()>;
}

pub trait SelfBuilder {
    type Output;

    fn new<S: Into<String>>(name: S) -> Self;

    fn build(&self) -> Result<Self::Output>;
}

pub async fn get_artifact_size(artifact: &dyn Artifact) -> Result<u64> {
    let memfs = artifact.extract().await?;
    memfs.size().await
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use eyre::Result;
    use smoosh::CompressionType;

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
            compression: CompressionType::Gzip,
            injections: vec![],
        };

        let tarball_artifact = tarball_producer.produce_from(&file_artifact).await?;

        assert_eq!(tarball_artifact.name(), "test.tar.gz");
        let tarball_path = PathBuf::from(tarball_artifact.name());
        assert!(tarball_path.exists());

        let file_producer = file::FileProducer {
            name: "test-file-producer".into(),
            path: "test".into(),
            injections: vec![],
            preserve_empty_directories: None,
        };

        let file_artifact = file_producer.produce_from(&tarball_artifact).await?;

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
