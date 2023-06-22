use std::path::PathBuf;

use bollard::image::CreateImageOptions;
use bollard::Docker;
use disk_drive::DiskDrive;
use eyre::Result;
use floppy_disk::mem::MemOpenOptions;
use floppy_disk::tokio_fs::TokioFloppyDisk;
use floppy_disk::{FloppyDisk, FloppyMetadata, FloppyOpenOptions};
use regex::Regex;
use smoosh::CompressionType;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_stream::StreamExt;
use tokio_util::codec::{BytesCodec, FramedRead};
use tracing::*;

use crate::artifact::memory::MemoryArtifact;
use crate::fs::{MemFS, TempDir};
use crate::util::config::Injection;

use super::tarball::{TarballArtifact, TarballProducer};
use super::{Artifact, ArtifactProducer, SelfBuilder, SelfValidation};

/// A Docker image.
///
/// ## Caveats
///
/// - Will currently always attempt to pull the provided image if needed
/// - Will only unpack the first set of layers in a Docker image
///
/// TODO: Preserve image config
#[derive(Debug, Clone)]
pub struct DockerArtifact {
    pub name: String,
    pub image: String,
}

#[async_trait::async_trait]
impl Artifact for DockerArtifact {
    fn name(&self) -> &str {
        &self.name
    }

    async fn extract(&self) -> Result<MemFS> {
        let docker = Docker::connect_with_local_defaults()?;
        let (image, tag) = split_image_name_into_repo_and_tag(&self.image);

        info!("attempting to pull {}...", self.image);
        // Attempt to download the image
        let mut pull = docker.create_image(
            Some(CreateImageOptions {
                from_image: image,
                tag,
                ..Default::default()
            }),
            None,
            None,
        );
        while let Some(info) = pull.next().await {
            let info = info?;
            info!("pulling {:?}: {:?}", image, info.progress);
        }

        // Export image to a TAR file
        let image_tar_export = TempDir::new().await?;

        info!("exporting to tarball...");
        let mut export = docker.export_image(&self.image);
        let export_name = format!("{}.tar", self.name.replace(['/', ':', ' '], "_"));
        let export_path = image_tar_export.path_view().join(&export_name);
        tokio::fs::create_dir_all(export_path.parent().unwrap())
            .await
            .unwrap();
        let mut file = tokio::fs::File::create(&export_path).await.unwrap();
        while let Some(chunk) = export.next().await {
            let chunk = chunk.unwrap();
            file.write_all(&chunk).await.unwrap();
            file.sync_all().await.unwrap();
        }

        // Docker exports a tarball of tarballs of layers

        // Extract the tarball into memory
        let basic_tar_memfs = TarballArtifact {
            name: self.name.clone(),
            path: export_path,
        }
        .extract()
        .await?;
        let basic_tar_fs = &*basic_tar_memfs;

        tokio::fs::remove_dir_all(&image_tar_export).await?;

        // Collect layers
        info!("gathering docker layers...");
        let mut manifest = MemOpenOptions::new()
            .read(true)
            .open(basic_tar_fs, "/manifest.json")
            .await?;
        let mut buf = String::new();
        manifest.read_to_string(&mut buf).await?;
        let manifest: serde_json::Value = serde_json::from_str(&buf)?;
        let layers: Vec<&str> = manifest
            .as_array()
            .unwrap()
            .get(0)
            .unwrap()
            .get("Layers")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();

        info!("extracting docker layers into memfs...");
        let fs = MemFS::new();
        let host = TokioFloppyDisk::new(Some(image_tar_export.path_view()));
        host.create_dir("/").await?;

        info!("copying base tarball contents to host...");
        DiskDrive::copy_between(basic_tar_fs, &host).await?;

        for layer in layers {
            debug!("copying layer: {layer}");
            let layer_path = image_tar_export.path_view().join(layer);
            let m = host.metadata(PathBuf::from("/").join(layer)).await?;
            dbg!(m.len());
            let layer_memfs = TarballArtifact {
                name: self.name.clone(),
                path: layer_path,
            }
            .extract()
            .await?;
            DiskDrive::copy_between(&*layer_memfs, &*fs).await?;
        }

        Ok(fs)
    }

    fn try_clone(&self) -> Result<Box<dyn Artifact>> {
        Ok(Box::new(self.clone()))
    }

    fn paths(&self) -> Option<Vec<PathBuf>> {
        None
    }
}

#[async_trait::async_trait]
impl SelfValidation for DockerArtifact {
    async fn validate(&self) -> Result<()> {
        Ok(())
    }
}

pub struct DockerArtifactBuilder {
    pub name: String,
    pub image: String,
}

#[allow(unused)]
impl DockerArtifactBuilder {
    pub fn image<S: Into<String>>(mut self, image: S) -> Self {
        self.image = image.into();
        self
    }
}

impl SelfBuilder for DockerArtifactBuilder {
    type Output = DockerArtifact;

    fn new<S: Into<String>>(name: S) -> Self {
        Self {
            name: name.into(),
            image: "".into(),
        }
    }

    fn build(&self) -> Result<Self::Output> {
        Ok(DockerArtifact {
            name: self.name.clone(),
            image: self.image.clone(),
        })
    }
}

/// Create a Docker image with the given name from an artifact, optionally
/// building the final image from another base image.
///
/// ## Caveats
///
/// - Will currently always attempt to pull the base image
/// - Does not support changes other than setting the `CMD`
#[derive(Debug, Clone)]
pub struct DockerProducer {
    pub name: String,
    pub image: String,
    pub base_image: Option<String>,
    pub cmd: Option<Vec<String>>,
    pub injections: Vec<Injection>,
}

#[async_trait::async_trait]
impl ArtifactProducer for DockerProducer {
    type Output = DockerArtifact;

    fn name(&self) -> &str {
        &self.name
    }

    fn injections(&self) -> &[Injection] {
        &self.injections
    }

    async fn can_produce_from(&self, _previous: &dyn Artifact) -> Result<()> {
        TokioFloppyDisk::new(None)
            .metadata("/var/run/docker.sock")
            .await
            .map(|_| ())
            .map_err(|e| e.into())
    }

    async fn produce_from(&self, previous: &dyn Artifact) -> Result<DockerArtifact> {
        // Produce a tarball artifact from the previous artifact
        let tmp = TempDir::new().await?;
        let tarball_path = tmp.path_view().join("image.tar");

        let tarball = if let Some(base_image) = &self.base_image {
            // If we have a base image, we need to build a new image on top of it
            // by importing the tarball into Docker and then exporting it again.
            // This is because Docker doesn't support importing a tarball of
            // layers directly.

            let merged_fs = {
                let out = MemFS::new();

                let base_fs = DockerArtifact {
                    name: self.name.clone(),
                    image: base_image.clone(),
                }
                .extract()
                .await?;

                let added_fs = previous.extract().await?;

                DiskDrive::copy_between(&*base_fs, &*out).await?;
                DiskDrive::copy_between(&*added_fs, &*out).await?;

                out
            };

            TarballProducer {
                name: self.name.clone(),
                path: tarball_path.clone(),
                compression: CompressionType::None,
                injections: self.injections.clone(),
            }
            .produce_from(&MemoryArtifact {
                name: self.name.clone(),
                fs: merged_fs,
            })
            .await?
        } else {
            // Otherwise, we can just import the tarball directly into Docker
            TarballProducer {
                name: self.name.clone(),
                path: tarball_path.clone(),
                compression: CompressionType::None,
                injections: self.injections.clone(),
            }
            .produce_from(previous)
            .await?
        };

        // Import the tarball into Docker
        let (image, tag) = split_image_name_into_repo_and_tag(&self.image);
        let docker = Docker::connect_with_local_defaults()?;
        let docker_cmd = {
            if let Some(docker_cmd) = self.cmd.clone() {
                let docker_cmd = format!("CMD {}", serde_json::to_string(&docker_cmd)?);
                debug!("docker_cmd = {docker_cmd}");
                Some(vec!["ENV DEBUG=true".into(), docker_cmd])
            } else {
                None
            }
        };
        debug!("docker_cmd = {docker_cmd:?}");
        let options = CreateImageOptions {
            from_src: "-".to_string(),
            repo: image.into(),
            changes: None, // docker_cmd,
            tag: tag.into(),
            ..Default::default()
        };

        let file = File::open(tarball.path)
            .await
            .map(|file| FramedRead::new(file, BytesCodec::new()))?;
        let req_body = hyper::body::Body::wrap_stream(file);

        let mut stream = docker.create_image(Some(options), Some(req_body), None);

        while let Some(progress) = stream.next().await {
            let progress = progress?;
            if let Some(status) = progress.status {
                info!("docker import: {}", status);
            }
        }

        tokio::fs::remove_dir_all(&tmp).await?;

        Ok(DockerArtifact {
            name: self.name.clone(),
            image: self.image.clone(),
        })
    }
}

#[async_trait::async_trait]
impl SelfValidation for DockerProducer {
    async fn validate(&self) -> Result<()> {
        // validate self.image format

        let docker_image_name_with_tag_and_repo_regex =
            Regex::new(r"^(?P<repo>[a-z0-9]+(?:[._-][a-z0-9]+)*/)?(?P<name>[a-z0-9]+(?:[._-][a-z0-9]+)*):(?P<tag>[a-z0-9]+(?:[._-][a-z0-9]+)*)$")
                .unwrap();

        let mut errors = vec![];

        if !docker_image_name_with_tag_and_repo_regex.is_match(&self.image) {
            errors.push(format!(
                "Docker image name with tag is invalid: {}, must match {docker_image_name_with_tag_and_repo_regex}",
                self.image
            ));
        }

        if let Some(base_image) = &self.base_image {
            if !docker_image_name_with_tag_and_repo_regex.is_match(base_image) {
                errors.push(format!(
                    "Docker base image name with tag is invalid: {}, must match {docker_image_name_with_tag_and_repo_regex}",
                    base_image
                ));
            }
        }

        if !errors.is_empty() {
            return Err(eyre::eyre!(
                "Docker producer is invalid:\n{}",
                errors.join("\n")
            ));
        }

        Ok(())
    }
}

pub struct DockerProducerBuilder {
    name: String,
    image: String,
    base_image: Option<String>,
    entrypoint: Option<Vec<String>>,
    injections: Vec<Injection>,
}

#[allow(unused)]
impl DockerProducerBuilder {
    pub fn image<S: Into<String>>(mut self, image: S) -> Self {
        self.image = image.into();
        self
    }

    pub fn base_image<S: Into<String>>(mut self, base_image: S) -> Self {
        self.base_image = Some(base_image.into());
        self
    }

    pub fn entrypoint(mut self, entrypoint: Vec<String>) -> Self {
        self.entrypoint = Some(entrypoint);
        self
    }

    pub fn inject(mut self, injection: Injection) -> Self {
        self.injections.push(injection);
        self
    }
}

impl SelfBuilder for DockerProducerBuilder {
    type Output = DockerProducer;

    fn new<S: Into<String>>(name: S) -> Self {
        Self {
            name: name.into(),
            image: "".into(),
            base_image: None,
            entrypoint: None,
            injections: vec![],
        }
    }

    fn build(&self) -> Result<Self::Output> {
        Ok(DockerProducer {
            name: self.name.clone(),
            image: self.image.clone(),
            base_image: self.base_image.clone(),
            cmd: self.entrypoint.clone(),
            injections: self.injections.clone(),
        })
    }
}

fn split_image_name_into_repo_and_tag(name: &str) -> (&str, &str) {
    if let Some((image, tag)) = name.split_once(':') {
        (image, tag)
    } else {
        (name, "latest")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use eyre::Result;

    #[ctor::ctor]
    fn init() {
        crate::util::test_init();
    }

    #[tokio::test]
    async fn test_docker_artifact_works() -> Result<()> {
        let artifact = DockerArtifact {
            name: "alpine-artifact".into(),
            image: "alpine:latest".to_string(),
        };
        {
            let fs = artifact.extract().await?;
            assert!(MemOpenOptions::new()
                .read(true)
                .open(&*fs, "/bin/sh")
                .await
                .is_ok());
        }

        let new_image = "peckish-dev/repackaged".to_string();
        let producer = DockerProducer {
            name: "docker image producer".into(),
            image: new_image.clone(),
            base_image: None,
            cmd: None,
            injections: vec![],
        };

        producer.produce_from(&artifact).await?;

        let docker = Docker::connect_with_local_defaults()?;

        assert!(docker.inspect_image(&new_image).await.is_ok());

        docker.remove_image(&new_image, None, None).await?;

        Ok(())
    }
}
