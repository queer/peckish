use color_eyre::eyre::Result;
use peckish::prelude::builder::*;
use peckish::prelude::*;

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

    let tarball_artifact = tarball_producer.produce_from(&file_artifact).await?;

    println!("tar t -vf {}", tarball_artifact.path.display());

    Ok(())
}
