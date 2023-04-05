//! Home for all frontends & UIs

pub mod console;
pub mod socket_server;

use crate::{
    runtime::Runtime,
    config::{Config, ExtendedOption, UiOptions},
    frontend::socket_server::SocketServer,
};
use tokio::sync::RwLock;
use log::{debug};


pub async fn async_run(runtime: &RwLock<Runtime>, config: &Config) -> Result<(), Box<dyn std::error::Error + Sync + Send>> {
    match config.ui {
        ExtendedOption::Enabled(ui) => match ui {
            UiOptions::Console(job) => console::async_run(&job, runtime, &config).await,
        }
        _ => panic!("BUG! empty `config.ui`"),
    }
}

pub fn run(runtime: &RwLock<Runtime>, config: &Config) -> Result<(), Box<dyn std::error::Error>> {
    match config.ui {
        ExtendedOption::Enabled(ui) => match ui {
            UiOptions::Console(job) => console::run(&job, runtime, &config),
        }
        _ => panic!("BUG! empty `config.ui`"),
    }
}

/// signals background (async Tokio) tasks that a graceful shutdown was requested
pub async fn shutdown_tokio_services(runtime: &RwLock<Runtime>) -> Result<(), Box<dyn std::error::Error + Sync + Send>> {

    debug!("Program logic is asking for a graceful shutdown...");

    tokio::join!(

        // shutdown socket server
        Runtime::do_if_socket_server_is_present(runtime, |socket_server| Box::pin(async move {
            socket_server.shutdown();
        })),

    );

    Ok(())
}

pub fn sync_shutdown_tokio_services(runtime: &RwLock<Runtime>) -> Result<(), Box<dyn std::error::Error + Sync + Send>> {
    runtime.blocking_read().tokio_runtime.as_ref().unwrap()
        .block_on(shutdown_tokio_services(runtime))
}