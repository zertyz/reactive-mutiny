//! see [super]

use std::time::Duration;
use crate::{
    runtime::Runtime,
    config::{Config, ExtendedOption},
};
use tokio::sync::RwLock;
use log::{info};


/// Runs the service this application provides
pub async fn long_runner(_runtime: &RwLock<Runtime>, _config: &Config) -> Result<(), Box<dyn std::error::Error>> {
    info!("HERE YOU WOULD START YOUR SERVICE. For now, we'll sleep for 3 min then quit");
    tokio::time::sleep(Duration::from_secs(180)).await;
    info!("DEMO DAEMON IS OVER. Application will now shutdown gracefully");
    Ok(())
}

/// Inspects & shows the effective configs & runtime used by the application
pub async fn check_config(runtime: &RwLock<Runtime>, config: &Config) -> Result<(), Box<dyn std::error::Error>> {
    println!("Effective Config:  {:#?}", config);
    let runtime = runtime.read().await;
    #[derive(Debug)]
    struct SerializableRuntime<'a> {
        executable_path:       &'a str,
        web_started:           bool,
        server_socket_started: bool,
        telegram_started:      bool,
    }
    let mut web_started           = false;
    let mut server_socket_started = false;
    let mut telegram_started      = false;
    if let ExtendedOption::Enabled(services) = &config.services {
        web_started           = services.web.is_enabled();
        server_socket_started = false;
        telegram_started      = services.telegram.is_enabled();
    }
    println!("Effective Runtime: {:#?}", SerializableRuntime {
        executable_path:  &runtime.executable_path,
        web_started,
        server_socket_started,
        telegram_started,
    });
    Ok(())
}