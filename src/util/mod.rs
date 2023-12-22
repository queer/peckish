use std::path::Path;

use eyre::{eyre, Result};
use sha2::Sha256;
use thiserror::Error;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, BufReader};
use tracing::*;

pub mod config;

#[derive(Error, Debug)]
pub enum Fix {
    #[error(transparent)]
    Anyhow(#[from] anyhow::Error),

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

#[cfg(test)]
#[allow(unused_must_use, unknown_lints, clippy::needless_if)]
pub fn test_init() {
    // std::env::set_var("RUST_LOG", "DEBUG");
    std::env::set_var("RUST_BACKTRACE", "full");
    std::panic::catch_unwind(|| {
        if color_eyre::install().is_err() {}
        let subscriber = tracing_subscriber::FmtSubscriber::builder()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .finish();
        tracing::subscriber::set_global_default(subscriber)
            .expect("setting default subscriber failed");
    });
}

pub fn get_current_time() -> Result<u64> {
    if let Ok(source_date_epoch) = std::env::var("SOURCE_DATE_EPOCH") {
        let source_date_epoch = source_date_epoch.parse::<u64>()?;
        let current_time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs();
        if source_date_epoch > current_time {
            return Err(eyre!("SOURCE_DATE_EPOCH is set to a time in the future"));
        }
        Ok(source_date_epoch)
    } else {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs();
        Ok(now)
    }
}

pub async fn sha256_digest(path: &Path) -> Result<String> {
    let input = File::open(path).await?;
    let mut reader = BufReader::new(input);

    let digest = {
        use sha2::Digest;
        let mut hasher = Sha256::new();
        let mut buffer = [0; 1024];
        loop {
            let count = reader.read(&mut buffer).await?;
            if count == 0 {
                break;
            }
            hasher.update(&buffer[..count]);
        }
        hasher.finalize()
    };
    Ok(format!("{:x}", digest))
}

pub fn sha256_digest_string<S: Into<String>>(string: S) -> Result<String> {
    let digest = {
        use sha2::Digest;
        let mut hasher = Sha256::new();
        hasher.update(string.into().as_bytes());
        hasher.finalize()
    };
    Ok(format!("{:x}", digest))
}
