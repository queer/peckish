use std::path::PathBuf;

use crate::artifact::tarball::{TarballArtifact, TarballProducerBuilder};
use crate::artifact::SelfBuilder;
use crate::fs::{MemFS, TempDir};
use crate::util::config::Injection;

use super::{Artifact, ArtifactProducer, SelfValidation};

use disk_drive::DiskDrive;
use eyre::Result;
use flop::tar::{TarFloppyDisk, TarOpenOptions};
use floppy_disk::mem::MemOpenOptions;
use floppy_disk::tokio_fs::TokioFloppyDisk;
use floppy_disk::{FloppyDisk, FloppyOpenOptions};
use oci_spec::image::{
    Arch, ConfigBuilder, DescriptorBuilder, ImageIndex, ImageIndexBuilder, ImageManifest,
    ImageManifestBuilder, MediaType, Os, PlatformBuilder,
};
use smoosh::CompressionType;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use tracing::{debug, info};

#[derive(Debug, Clone)]
pub struct OciArtifact {
    pub name: String,
    pub path: PathBuf,
}

fn blob_to_path<S: Into<String>>(digest: S) -> String {
    let digest = digest.into();
    let (algorithm, hash) = {
        let mut split = digest.splitn(2, ':');
        (split.next().unwrap(), split.next().unwrap())
    };
    let blob = format!("/blobs/{}/{}", algorithm, hash);
    blob
}

#[async_trait::async_trait]
impl Artifact for OciArtifact {
    fn name(&self) -> &str {
        &self.name
    }

    async fn extract(&self) -> Result<MemFS> {
        // Extract the tarball into memory
        info!("extracting oci image...");
        let oci_tar_fs = &*TarballArtifact {
            name: self.name.clone(),
            path: self.path.clone(),
        }
        .extract()
        .await?;

        let index = MemOpenOptions::new()
            .read(true)
            .open(oci_tar_fs, "/index.json")
            .await?;
        let index = ImageIndex::from_reader(index)?;

        // Read all manifests
        // TODO: This will explode if there's actually more than one...
        let fs = MemFS::new();
        for manifest in index.manifests() {
            let blob = blob_to_path(manifest.digest());
            debug!("reading blob: {blob}");
            let manifest = MemOpenOptions::new()
                .read(true)
                .open(oci_tar_fs, blob)
                .await?;
            let manifest = ImageManifest::from_reader(manifest)?;

            // Read all layers
            for layer in manifest.layers() {
                debug!("reading layer blob: {}", blob_to_path(layer.digest()));
                let layer_tmp_dir = TempDir::new().await?;
                match layer.media_type() {
                    oci_spec::image::MediaType::ImageLayer
                    | oci_spec::image::MediaType::ImageLayerGzip
                    | oci_spec::image::MediaType::ImageLayerZstd => {
                        // Copy actual image layers to the memfs
                        debug!("copying blob to memfs: {}", blob_to_path(layer.digest()));

                        let tmp_layer_tar = TokioFloppyDisk::new(Some(layer_tmp_dir.path_view()));
                        DiskDrive::copy_from_src_to_dest(
                            oci_tar_fs,
                            &tmp_layer_tar,
                            blob_to_path(layer.digest()),
                            "/layer.tar",
                        )
                        .await?;

                        let tar_disk =
                            TarFloppyDisk::open(layer_tmp_dir.path_view().join("layer.tar"))
                                .await?;

                        DiskDrive::copy_between(&tar_disk, &*fs).await?;
                    }
                    _ => {}
                }
            }
        }

        Ok(fs)
    }

    fn try_clone(&self) -> Result<Box<dyn Artifact>> {
        Ok(Box::new(self.clone()))
    }

    fn paths(&self) -> Option<Vec<PathBuf>> {
        Some(vec![self.path.clone()])
    }
}

#[async_trait::async_trait]
impl SelfValidation for OciArtifact {
    async fn validate(&self) -> Result<()> {
        let mut errors = vec![];

        if !self.path.exists() {
            errors.push(format!("path does not exist: {:?}", self.path));
        }

        if !self.path.is_file() {
            errors.push(format!("path is not a file: {:?}", self.path));
        }

        if !errors.is_empty() {
            return Err(eyre::eyre!(
                "oci artifact not valid:\n{}",
                errors.join("\n")
            ));
        }

        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct OciProducer {
    pub name: String,
    pub path: PathBuf,
    pub architecture: String,
    pub injections: Vec<Injection>,
}

#[async_trait::async_trait]
impl ArtifactProducer for OciProducer {
    type Output = OciArtifact;

    fn name(&self) -> &str {
        &self.name
    }

    fn injections(&self) -> &[Injection] {
        &self.injections
    }

    async fn produce_from(&self, previous: &dyn Artifact) -> Result<Self::Output> {
        // /index.json
        // /oci-layout
        // /blobs

        // Produce fs blob tarball
        let oci_layout = r#"{"imageLayoutVersion": "1.0.0"}"#;
        let blob_out_dir = TempDir::new().await?;
        let mut blob = TarballProducerBuilder::new(&self.name)
            .path(blob_out_dir.path_view().join("blob.tar.gz"))
            .compression(CompressionType::Gzip);

        for injection in self.injections() {
            debug!("applying injection {injection:?}");
            blob = blob.inject(injection.clone());
        }

        let blob = blob.build()?.produce_from(previous).await?;
        let blob_sha256 = crate::util::sha256_digest(&blob.path).await?;
        let blob_path = blob_to_path(format!("sha256:{blob_sha256}"));

        // Produce layer blob descriptor
        let layer_descriptor = DescriptorBuilder::default()
            .size(previous.extract().await?.size().await? as i64)
            .platform(
                PlatformBuilder::default()
                    .architecture(match self.architecture.as_str() {
                        "amd64" => Arch::Amd64,
                        "aarch64" => Arch::ARM64,
                        _ => unimplemented!(),
                    })
                    .os(Os::Linux)
                    .build()
                    .expect("build amd64 platform"),
            )
            .digest(format!("sha256:{blob_sha256}"))
            .media_type(MediaType::ImageLayerZstd)
            .build()
            .expect("failed building layer descriptor");

        // Produce config + descriptor
        let config = ConfigBuilder::default().build()?;
        let config_string = serde_json::to_string(&config)?;
        let config_sha256 = crate::util::sha256_digest_string(&config_string)?;
        let config_descriptor = DescriptorBuilder::default()
            .size(config_string.len() as i64)
            .media_type(MediaType::ImageConfig)
            .digest(format!("sha256:{config_sha256}"))
            .build()?;
        let config_descriptor_path = blob_to_path(config_descriptor.digest().to_string());

        // Produce image manifest
        let image_manifest = ImageManifestBuilder::default()
            .schema_version(2u32)
            .layers(vec![layer_descriptor])
            .config(config_descriptor)
            .build()?;
        let image_manifest_sha256 = {
            use sha2::Digest;
            let mut hasher = sha2::Sha256::new();
            hasher.update(image_manifest.to_string()?.as_bytes());
            let digest = hasher.finalize();
            format!("{:x}", digest)
        };

        // Produce image manifest descriptor
        let image_manifest_descriptor = DescriptorBuilder::default()
            .size(image_manifest.to_string()?.len() as i64)
            .media_type(MediaType::ImageManifest)
            .digest(format!("sha256:{image_manifest_sha256}"))
            .build()?;
        let image_manifest_descriptor_path =
            blob_to_path(format!("sha256:{image_manifest_sha256}"));

        // Pull it all together into the index!
        let index = ImageIndexBuilder::default()
            .schema_version(2u32)
            .manifests(vec![image_manifest_descriptor.clone()])
            .build()?;

        // Create tarball

        let oci_tar = TarFloppyDisk::open(self.path.clone()).await?;
        oci_tar.create_dir_all("/blobs/sha256").await?;

        let mut layer_handle = TarOpenOptions::new()
            .create(true)
            .write(true)
            .open(&oci_tar, &blob_path)
            .await?;
        tokio::io::copy(&mut File::open(blob.path).await?, &mut layer_handle).await?;
        debug!("write layer blob {blob_path}");

        let mut image_manifest_descriptor_handle = TarOpenOptions::new()
            .create(true)
            .write(true)
            .open(&oci_tar, &image_manifest_descriptor_path)
            .await?;
        image_manifest_descriptor_handle
            .write_all(serde_json::to_string(&image_manifest)?.as_bytes())
            .await?;
        debug!("write image descriptor blob {image_manifest_descriptor_path}");

        let mut config_descriptor_handle = TarOpenOptions::new()
            .create(true)
            .write(true)
            .open(&oci_tar, &config_descriptor_path)
            .await?;
        config_descriptor_handle
            .write_all(config_string.as_bytes())
            .await?;
        debug!("write config descriptor blob {config_descriptor_path}");

        let mut oci_layout_handle = TarOpenOptions::new()
            .create(true)
            .write(true)
            .open(&oci_tar, "/oci-layout")
            .await?;
        oci_layout_handle.write_all(oci_layout.as_bytes()).await?;

        let mut index_handle = TarOpenOptions::new()
            .create(true)
            .write(true)
            .open(&oci_tar, "/index.json")
            .await?;
        index_handle
            .write_all(index.to_string()?.as_bytes())
            .await?;

        oci_tar.close().await?;

        Ok(OciArtifact {
            name: self.name.clone(),
            path: self.path.clone(),
        })
    }
}

#[async_trait::async_trait]
impl SelfValidation for OciProducer {
    async fn validate(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        if TokioFloppyDisk::new(None)
            .metadata(&self.path)
            .await
            .is_err()
        {
            Ok(())
        } else {
            Err(eyre::eyre!(
                "cannot produce artifact '{}': path already exists: {}",
                self.name,
                self.path.display()
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use eyre::Result;
    use floppy_disk::{FloppyFile, FloppyMetadata};

    use crate::fs::test_utils::Fixture;

    use super::*;

    #[tokio::test]
    async fn test_oci_artifact() -> Result<()> {
        let oci_tarball = Fixture::new("oci.tar").await;
        let oci_artifact = OciArtifact {
            name: "test".to_string(),
            path: oci_tarball.path_view(),
        };

        let fs = oci_artifact.extract().await?;
        let handle = MemOpenOptions::new()
            .read(true)
            .open(&*fs, "/usr/local/bin/podman_hello_world")
            .await?;

        let metadata = handle.metadata().await?;
        assert!(metadata.is_file());

        Ok(())
    }

    #[tokio::test]
    async fn test_oci_producer() -> Result<()> {
        let oci_tarball = Fixture::new("oci.tar").await;
        let oci_artifact = OciArtifact {
            name: "test".into(),
            path: oci_tarball.path_view(),
        };

        let tmp_dir = TempDir::new().await?;
        let oci_producer = OciProducer {
            name: "test".into(),
            path: tmp_dir.path_view().join("oci.tar"),
            architecture: "amd64".into(),
            injections: vec![],
        };

        let oci_artifact = oci_producer.produce_from(&oci_artifact).await?;

        let memfs = oci_artifact.extract().await?;
        let handle = MemOpenOptions::new()
            .read(true)
            .open(&*memfs, "/usr/local/bin/podman_hello_world")
            .await?;

        let metadata = handle.metadata().await?;
        assert!(metadata.is_file());

        Ok(())
    }
}
