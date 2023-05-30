use std::path::PathBuf;

use clap::Parser;
use color_eyre::Result;
use tracing::*;

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

    #[arg(
        short = 'r',
        long = "report",
        help = "Name of the file to generate artifact file output report to."
    )]
    report_file: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    if std::env::var("RUST_LOG").is_err() {
        std::env::set_var("RUST_LOG", "info");
    }

    color_eyre::config::HookBuilder::new()
        .issue_url(concat!(env!("CARGO_PKG_REPOSITORY"), "/issues/new"))
        .add_issue_metadata("version", env!("CARGO_PKG_VERSION"))
        .install()?;
    let subscriber = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .compact()
        .finish();
    tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber failed");
    tracing_log::LogTracer::init()?;

    let args = Input::parse();

    debug!("starting peckish");

    let config = PeckishConfig::load(args.config_file).await?;

    Pipeline::new(args.report_file).run(config).await?;

    Ok(())
}
