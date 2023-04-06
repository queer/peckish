use color_eyre::eyre::Result;
use peckish::prelude::builder::*;
use peckish::prelude::*;

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

    let tarball_artifact = tarball_producer.produce_from(&file_artifact).await?;

    println!("tar t -vf {}", tarball_artifact.path.display());

    Ok(())
}
