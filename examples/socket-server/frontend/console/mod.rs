use crate::{runtime::Runtime, config::{Config, Jobs}, logic, frontend};
use tokio::sync::RwLock;


pub async fn async_run(job: &Jobs, runtime: &RwLock<Runtime>, config: &Config) -> Result<(), Box<dyn std::error::Error + Sync + Send>> {
    match job {
        Jobs::CheckConfig => logic::check_config(runtime, config).await?,
        Jobs::Daemon      => logic::long_runner(runtime, config).await?,
    }
    frontend::shutdown_tokio_services(runtime).await
}

/// on this example, our app's console frontend only uses Async Rust -- so we don't do nothing here
pub fn run(_job: &Jobs, _runtime: &RwLock<Runtime>, _config: &Config) -> Result<(), Box<dyn std::error::Error>> {
    Ok(())
}