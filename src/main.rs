use clap::Parser;
use color_eyre::Result;
use log::*;

use crate::pipeline::Pipeline;
use crate::util::config::PeckishConfig;

mod artifact;
mod fs;
mod pipeline;
mod util;

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(clap::Parser)]
#[command(
    name = "peckish",
    display_name = "peckish",
    about = "peckish repackages software artifacts!",
    version = VERSION,
)]
struct Input {
    #[arg(short = 'c', long = "config")]
    config_file: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    if std::env::var("RUST_LOG").is_err() {
        std::env::set_var("RUST_LOG", "info");
    }

    color_eyre::install()?;
    pretty_env_logger::init();

    let args = Input::parse();

    debug!("starting peckish");

    let config = PeckishConfig::load(args.config_file).await?;

    Pipeline::new(false).run(config).await?;

    Ok(())
}
