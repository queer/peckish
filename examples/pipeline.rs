use color_eyre::eyre::Result;
use peckish::prelude::builder::*;
use peckish::prelude::pipeline::*;
use tracing::info;

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;
    let subscriber = tracing_subscriber::FmtSubscriber::builder()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .finish();
    tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber failed");

    let file_artifact = FileArtifactBuilder::new("example file artifact")
        .add_path("./examples/data/a")
        .build()?;

    let tarball_producer = TarballProducerBuilder::new("example tarball producer")
        .path("test.tar.gz")
        .build()?;

    let debian_producer = DebProducerBuilder::new("example debian producer")
        .path("test.deb")
        .package_name("test")
        .package_maintainer("me <me@example.com>")
        .package_version("0.0.1-1")
        .package_description("test package")
        .package_architecture("amd64")
        .build()?;

    let config = PeckishConfig {
        input: ConfiguredArtifact::File(file_artifact),
        output: vec![
            ConfiguredProducer::Tarball(tarball_producer),
            ConfiguredProducer::Deb(debian_producer),
        ],
        chain: false,
    };

    let pipeline = Pipeline::default();

    let out = pipeline.run(config).await?;

    info!("produced {} artifacts", out.len());

    Ok(())
}
