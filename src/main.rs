use color_eyre::Result;
use log::*;

use crate::pipeline::Pipeline;
use crate::util::config::PeckishConfig;

mod artifact;
mod pipeline;
mod util;

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;
    pretty_env_logger::init();
    debug!("starting peckish");

    let config = PeckishConfig::load(Some("./peckish.yaml".into())).await?;

    Pipeline::new().run(config).await?;

    Ok(())
}
