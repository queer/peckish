use color_eyre::Result;

use crate::util::MemoryFS;

pub mod file;
pub mod tarball;

/// An artifact is the result of some build process.
#[async_trait::async_trait]
pub trait Artifact: Send + Sync {
    fn name(&self) -> &String;
    fn description(&self) -> String;

    /// Extract this artifact into a virtual filesystem. Used for manipulating
    /// the artifact's contents.
    async fn extract(&self) -> Result<MemoryFS>;
}

/// An artifact producer takes in the previous artifact and produces a new one.
#[async_trait::async_trait]
pub trait ArtifactProducer {
    type Output: Artifact;

    fn name(&self) -> &String;

    /// Produce a new artifact, given a previous artifact.
    async fn produce(&self, previous: &dyn Artifact) -> Result<Self::Output>;
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use color_eyre::Result;

    use crate::util::Fix;

    use super::*;

    #[tokio::test]
    async fn test_basic_transform_works() -> Result<()> {
        let file_artifact = file::FileArtifact {
            name: "Cargo.toml".into(),
            paths: vec![PathBuf::from("Cargo.toml")],
        };

        let tarball_producer = tarball::TarballProducer {
            name: "test-tarball-producer".into(),
            path: "test.tar.gz".into(),
        };

        let tarball_artifact = tarball_producer.produce(&file_artifact).await?;

        assert_eq!(tarball_artifact.name(), "test.tar.gz");
        let tarball_path = PathBuf::from(tarball_artifact.name());
        assert!(tarball_path.exists());

        let file_producer = file::FileProducer {
            name: "test-file-producer".into(),
            path: "test".into(),
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
