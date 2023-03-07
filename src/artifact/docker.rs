use std::collections::HashMap;
use std::path::PathBuf;

use bollard::image::CreateImageOptions;
use bollard::Docker;
use color_eyre::Result;
use log::*;
use rsfs::GenFS;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use tokio_stream::StreamExt;
use tokio_util::codec::{BytesCodec, FramedRead};

use crate::artifact::copy_files_from_paths_to_memfs;
use crate::util::config::Injection;
use crate::util::{create_tmp_dir, MemoryFS};

use super::tarball::{TarballArtifact, TarballProducer};
use super::{Artifact, ArtifactProducer};

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

    fn description(&self) -> &str {
        "A docker image"
    }

    async fn extract(&self) -> Result<MemoryFS> {
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
            info!("pulling {:?}: {:?}", info.id, info.progress);
        }

        // Export image to a TAR file
        let tmp = create_tmp_dir().await?;

        let mut export = docker.export_image(&self.image);
        let export_name = format!("{}.tar", self.name);
        let export_path = tmp.join(&export_name);
        let export_path_clone = export_path.clone();
        let join_handle = tokio::spawn(async move {
            tokio::fs::create_dir_all(export_path_clone.parent().unwrap())
                .await
                .unwrap();
            let mut file = tokio::fs::File::create(export_path_clone).await.unwrap();
            while let Some(chunk) = export.next().await {
                let chunk = chunk.unwrap();
                file.write_all(&chunk).await.unwrap();
                file.sync_all().await.unwrap();
            }
        });
        join_handle.await?;

        // Docker exports a tarball of tarballs of layers

        // Extract the tarball into memory
        let basic_tar_fs = TarballArtifact {
            name: self.name.clone(),
            path: export_path,
        }
        .extract()
        .await?;

        tokio::fs::remove_dir_all(&tmp).await?;

        // Collect layers
        let manifest = basic_tar_fs.open_file("/manifest.json")?;
        let manifest: serde_json::Value = serde_json::from_reader(manifest)?;
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

        let tmp = create_tmp_dir().await?;
        for layer in layers {
            // For each layer, extract it into the tmp directory.
            let layer_tar = basic_tar_fs.open_file(&format!("/{}", layer))?;
            let tmp_clone = tmp.clone();
            let join_handle = tokio::spawn(async move {
                let mut layer_tar = tar::Archive::new(layer_tar);
                layer_tar.unpack(&tmp_clone).unwrap();
            });
            join_handle.await?;
        }

        // Read Docker layers into the memfs
        // We don't reuse the file artifact here because we need to control how
        // the file paths are computed.

        let fs = MemoryFS::new();

        debug!("copying docker layers to memfs!");

        let mut memfs_paths = HashMap::new();
        memfs_paths.insert(tmp.to_path_buf(), PathBuf::from("/"));
        copy_files_from_paths_to_memfs(&memfs_paths, &fs).await?;

        tokio::fs::remove_dir_all(&tmp).await?;

        Ok(fs)
    }
}

#[derive(Debug, Clone)]
pub struct DockerProducer {
    pub name: String,
    pub image: String,
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
        let tmp = create_tmp_dir().await?;
        let tarball_path = tmp.join("image.tar");
        let tarball = TarballProducer {
            name: self.name.clone(),
            path: tarball_path,
            injections: self.injections.clone(),
        }
        .produce(previous)
        .await?;

        // Import the tarball into Docker
        let (image, tag) = split_image_name_into_repo_and_tag(&self.image);
        let docker = Docker::connect_with_local_defaults()?;
        let options = CreateImageOptions {
            from_src: "-",
            repo: image,
            tag,
            ..Default::default()
        };

        let file = File::open(tarball.path)
            .await
            .map(|file| FramedRead::new(file, BytesCodec::new()))?;
        let req_body = hyper::body::Body::wrap_stream(file);

        let mut stream = docker.create_image(Some(options), Some(req_body), None);

        while let Some(progress) = stream.next().await {
            let progress = progress?;
            info!("importing {:?}: {:?}", progress.id, progress.progress);
        }

        tokio::fs::remove_dir_all(&tmp).await?;

        Ok(DockerArtifact {
            name: self.name.clone(),
            image: self.image.clone(),
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

    use color_eyre::Result;
    use rsfs::GenFS;

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
            assert!(fs.open_file(Path::new("/bin/sh")).is_ok());
        }

        let new_image = "peckish-dev/repackaged".to_string();
        let producer = DockerProducer {
            name: "docker iamge producer".into(),
            image: new_image.clone(),
            injections: vec![],
        };

        producer.produce(&artifact).await?;

        let docker = Docker::connect_with_local_defaults()?;

        assert!(docker.inspect_image(&new_image).await.is_ok());

        docker.remove_image(&new_image, None, None).await?;

        Ok(())
    }
}