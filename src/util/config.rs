use std::path::PathBuf;

use color_eyre::Result;
use serde::{Deserialize, Serialize};
use tokio::fs::File;
use tokio::io::AsyncReadExt;

use crate::artifact::file::{FileArtifact, FileProducer};
use crate::artifact::tarball::{TarballArtifact, TarballProducer};

#[derive(Debug)]
pub struct PeckishConfig {
    pub input: ConfiguredArtifact,
    pub output: Vec<ConfiguredProducer>,
}

impl PeckishConfig {
    pub async fn load(config: Option<String>) -> Result<Self> {
        let config_file: PathBuf = config.unwrap_or_else(|| "peckish.yaml".into()).into();
        let mut config_file = File::open(config_file).await?;
        let mut config_str = String::new();
        config_file.read_to_string(&mut config_str).await?;

        let config: InternalConfig = serde_yaml::from_str(&config_str)?;

        Ok(Self {
            input: config.input.into(),
            output: config.output.iter().map(|o| o.into()).collect(),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct InternalConfig {
    input: InputArtifact,
    output: Vec<OutputProducer>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum InputArtifact {
    File { name: String, paths: Vec<PathBuf> },
    Tarball { name: String, path: PathBuf },
}

// Safety: This is intended to be a one-way conversion
#[allow(clippy::from_over_into)]
impl Into<ConfiguredArtifact> for InputArtifact {
    fn into(self) -> ConfiguredArtifact {
        match self {
            InputArtifact::File { name, paths } => {
                ConfiguredArtifact::File(FileArtifact { name, paths })
            }
            InputArtifact::Tarball { name, path } => {
                ConfiguredArtifact::Tarball(TarballArtifact { name, path })
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum OutputProducer {
    File { name: String, path: PathBuf },
    Tarball { name: String, path: PathBuf },
}

// Safety: This is intended to be a one-way conversion
#[allow(clippy::from_over_into)]
impl Into<ConfiguredProducer> for &OutputProducer {
    fn into(self) -> ConfiguredProducer {
        match self {
            OutputProducer::File { name, path } => ConfiguredProducer::File(FileProducer {
                name: name.clone(),
                path: path.clone(),
            }),
            OutputProducer::Tarball { name, path } => {
                ConfiguredProducer::Tarball(TarballProducer {
                    name: name.clone(),
                    path: path.clone(),
                })
            }
        }
    }
}

#[derive(Debug, Clone)]
pub enum ConfiguredArtifact {
    File(FileArtifact),
    Tarball(TarballArtifact),
}

#[derive(Debug, Clone)]
pub enum ConfiguredProducer {
    File(FileProducer),
    Tarball(TarballProducer),
}
