use color_eyre::eyre::Result;
use peckish::prelude::builder::*;
use peckish::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;
    pretty_env_logger::init();

    let tarball_artifact = TarballArtifactBuilder::new("example tarball artifact")
        .path("./examples/example.tar.Zstd")
        .build()?;

    let file_producer = FileProducerBuilder::new("example file producer")
        .path("./test_output_pls_ignore")
        .build()?;

    let file_artifact = file_producer.produce_from(&tarball_artifact).await?;

    println!(
        "ls -lah {}",
        file_artifact
            .paths
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<String>>()
            .join(" ")
    );

    Ok(())
}
