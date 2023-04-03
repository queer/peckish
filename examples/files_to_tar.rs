use color_eyre::eyre::Result;
use peckish::prelude::builder::*;
use peckish::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;
    pretty_env_logger::init();

    let file_artifact = FileArtifactBuilder::new("example file artifact".into())
        .add_path("./examples/a".into())
        .build()?;

    let tarball_producer = TarballProducerBuilder::new("example tarball producer".into())
        .path("test.tar.gz".into())
        .build()?;

    let tarball_artifact = tarball_producer.produce(&file_artifact).await?;

    println!("tar t -vf {}", tarball_artifact.path.display());

    Ok(())
}
