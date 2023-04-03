use color_eyre::eyre::Result;
use log::info;
use peckish::prelude::builder::*;
use peckish::prelude::pipeline::*;

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;
    pretty_env_logger::init();

    let file_artifact = FileArtifactBuilder::new("example file artifact")
        .add_path("./examples/a")
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
        pipeline: false,
    };

    let pipeline = Pipeline::new();

    let out = pipeline.run(config).await?;

    info!("produced {} artifacts", out.len());

    Ok(())
}
