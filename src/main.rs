use color_eyre::Result;
use log::*;

use crate::pipeline::Pipeline;
use crate::util::config::PeckishConfig;

mod artifact;
mod fs;
mod pipeline;
mod util;

#[tokio::main]
async fn main() -> Result<()> {
    if std::env::var("RUST_LOG").is_err() {
        std::env::set_var("RUST_LOG", "info");
    }

    color_eyre::install()?;
    pretty_env_logger::init();

    debug!("starting peckish");

    let config = PeckishConfig::load(Some("./peckish.yaml".into())).await?;

    Pipeline::new(false).run(config).await?;

    Ok(())
}
