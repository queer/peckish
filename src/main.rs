use color_eyre::Result;
use util::config::PeckishConfig;

mod artifact;
mod util;

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;

    let config = PeckishConfig::load(Some("./peckish.yaml".into())).await?;

    dbg!(config);

    Ok(())
}
