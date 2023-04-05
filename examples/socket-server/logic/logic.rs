//! see [super]

use std::time::Duration;
use crate::{
    runtime::Runtime,
    config::{Config, ExtendedOption},
};
use tokio::sync::RwLock;
use log::{info};


/// Runs the service this application provides
pub async fn long_runner(runtime: &RwLock<Runtime>, _config: &Config) -> Result<(), Box<dyn std::error::Error + Sync + Send>> {
    let shutdown_waiter = Runtime::do_for_ogre_robot(runtime, |ogre_robot| Box::pin(async { ogre_robot.shutdown_waiter() })).await;
    shutdown_waiter.await;
    Ok(())
}

/// Inspects & shows the effective configs & runtime used by the application
pub async fn check_config(runtime: &RwLock<Runtime>, config: &Config) -> Result<(), Box<dyn std::error::Error + Sync + Send>> {
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