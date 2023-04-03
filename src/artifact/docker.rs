use bollard::image::CreateImageOptions;
use bollard::Docker;
use eyre::Result;
use log::*;
use regex::Regex;
use rsfs_tokio::GenFS;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_stream::StreamExt;
use tokio_util::codec::{BytesCodec, FramedRead};

use crate::artifact::file::{FileArtifact, FileProducer};
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
        let tmp = TempDir::new().await?;

        let mut export = docker.export_image(&self.image);
        let export_name = format!("{}.tar", self.name);
        let export_path = tmp.path_view().join(&export_name);
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
        let basic_tar_fs = basic_tar_memfs.as_ref();

        tokio::fs::remove_dir_all(&tmp).await?;

        // Collect layers
        let mut manifest = basic_tar_fs.open_file("/manifest.json").await?;
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

        let tmp = TempDir::new().await?;
        for layer in layers {
            // For each layer, extract it into the tmp directory.
            let layer_tar = basic_tar_fs.open_file(&format!("/{}", layer)).await?;
            let tmp_clone = tmp.path_view();
            let mut layer_tar = tokio_tar::Archive::new(layer_tar);
            layer_tar.unpack(&tmp_clone).await?;
        }

        // Read Docker layers into the memfs
        // We don't reuse the file artifact here because we need to control how
        // the file paths are computed.

        let fs = MemFS::new();

        debug!("copying docker layers to memfs!");

        let memfs_paths = vec![tmp.path_view()];
        fs.copy_files_from_paths(&memfs_paths, Some(tmp.path_view()))
            .await?;

        Ok(fs)
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
    pub fn image(mut self, image: String) -> Self {
        self.image = image;
        self
    }
}

impl SelfBuilder for DockerArtifactBuilder {
    type Output = DockerArtifact;

    fn new(name: String) -> Self {
        Self {
            name,
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
///
/// TODO: Rename `entrypoint` -> `cmd`
#[derive(Debug, Clone)]
pub struct DockerProducer {
    pub name: String,
    pub image: String,
    pub base_image: Option<String>,
    pub entrypoint: Option<Vec<String>>,
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

    async fn produce(&self, previous: &dyn Artifact) -> Result<DockerArtifact> {
        // Produce a tarball artifact from the previous artifact
        let tmp = TempDir::new().await?;
        let tarball_path = tmp.path_view().join("image.tar");

        let tarball = if let Some(base_image) = &self.base_image {
            // If we have a base image, we need to build a new image on top of it
            // by importing the tarball into Docker and then exporting it again.
            // This is because Docker doesn't support importing a tarball of
            // layers directly.

            let tmp = TempDir::new().await?;

            {
                // FileProducer extract Docker artifact of self.base_image into tmp
                debug!(
                    "extracting base image {base_image} to {:?}",
                    tmp.path_view()
                );
                FileProducer {
                    name: self.name.clone(),
                    path: tmp.path_view(),
                    preserve_empty_directories: Some(true),
                    injections: vec![],
                }
                .produce(&DockerArtifact {
                    name: self.name.clone(),
                    image: base_image.clone(),
                })
                .await?;
            }

            {
                // FileProducer extract previous artifact into tmp
                debug!(
                    "extracting previous artifact {} to {:?}",
                    previous.name(),
                    tmp.path_view()
                );
                FileProducer {
                    name: self.name.clone(),
                    path: tmp.path_view(),
                    preserve_empty_directories: Some(true),
                    injections: vec![],
                }
                .produce(previous)
                .await?;
            }

            TarballProducer {
                name: self.name.clone(),
                path: tarball_path.clone(),
                injections: self.injections.clone(),
            }
            .produce(&FileArtifact {
                name: self.name.clone(),
                paths: vec![tmp.path_view()],
                strip_path_prefixes: Some(true),
                preserve_empty_directories: Some(true),
            })
            .await?
        } else {
            // Otherwise, we can just import the tarball directly into Docker
            TarballProducer {
                name: self.name.clone(),
                path: tarball_path.clone(),
                injections: self.injections.clone(),
            }
            .produce(previous)
            .await?
        };

        // Import the tarball into Docker
        let (image, tag) = split_image_name_into_repo_and_tag(&self.image);
        let docker = Docker::connect_with_local_defaults()?;
        let docker_cmd = {
            if let Some(docker_cmd) = self.entrypoint.clone() {
                let docker_cmd = format!("CMD {}", serde_json::to_string(&docker_cmd)?);
                debug!("docker_cmd = {docker_cmd}");
                Some(docker_cmd)
            } else {
                None
            }
        };
        debug!("docker_cmd = {docker_cmd:?}");
        let options = CreateImageOptions {
            from_src: "-".to_string(),
            repo: image.into(),
            changes: docker_cmd,
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
    pub fn image(mut self, image: String) -> Self {
        self.image = image;
        self
    }

    pub fn base_image(mut self, base_image: String) -> Self {
        self.base_image = Some(base_image);
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

    fn new(name: String) -> Self {
        Self {
            name,
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
            entrypoint: self.entrypoint.clone(),
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
    use std::path::Path;

    use super::*;

    use eyre::Result;
    use rsfs_tokio::GenFS;

    #[ctor::ctor]
    fn init() {
        crate::util::test_init();
    }

    #[tokio::test]
    async fn test_docker_artifact_works() -> Result<()> {
        let artifact = DockerArtifact {
            name: "alpine-artifact".into(),
            image: "alpine:3.13".to_string(),
        };
        {
            let fs = artifact.extract().await?;
            let fs = fs.as_ref();
            assert!(fs.open_file(Path::new("/bin/sh")).await.is_ok());
        }

        let new_image = "peckish-dev/repackaged".to_string();
        let producer = DockerProducer {
            name: "docker image producer".into(),
            image: new_image.clone(),
            base_image: None,
            entrypoint: None,
            injections: vec![],
        };

        producer.produce(&artifact).await?;

        let docker = Docker::connect_with_local_defaults()?;

        assert!(docker.inspect_image(&new_image).await.is_ok());

        docker.remove_image(&new_image, None, None).await?;

        Ok(())
    }
}
