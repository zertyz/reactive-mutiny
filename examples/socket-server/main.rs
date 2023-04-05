#![doc = include_str!("README.md")]


mod runtime;
mod config;
mod frontend;
mod command_line;
mod logic;

use crate::{
    runtime::Runtime,
    config::{
        APP_NAME,
        DEBUG,
        Config,
        UiOptions,
        ExtendedOption,
        config_ops,
    },
};
use std::{
    error::Error,
    sync::Arc,
    thread::{self, JoinHandle},
};
use std::borrow::BorrowMut;
use tokio::sync::RwLock;
use log::{debug, error, warn};
use owning_ref::ArcRef;


fn custom_sync_initialization(_runtime: &RwLock<Runtime>, _config: &Config) -> Result<(), Box<dyn Error>> {
    // nothing here, for now...
    Ok(())
}

fn sync_main(runtime: &RwLock<Runtime>, config: &Config) -> Result<(), Box<dyn Error>> {
    let result = frontend::run(runtime, config);
    debug!("App's sync main is done. Result: '{:?}'", result);
    result
}

async fn async_main(runtime: &RwLock<Runtime>, config: &Config) -> Result<(), Box<dyn Error>> {
    let result = frontend::async_run(runtime, config).await;
    debug!("App's async main is done. Result: '{:?}'", result);
    result
}

fn main() -> Result<(), Box<dyn Error>> {

    let command_line_options = command_line::parse_from_args();
    let config_file_options = load_configs();
    let effective_config = Arc::new(command_line::merge_config_file_and_command_line_options(config_file_options, command_line_options));
    let _logger_guard = setup_logging(&effective_config);
    let runtime = Arc::new(build_runtime());

    warn!("{} application started!", APP_NAME);
    debug!("Running 'custom_sync_initialization()':");
    custom_sync_initialization(&runtime, &effective_config).expect("Error in 'custom_sync_initialization()'");

    let tokio_join_handle = start_tokio_runtime_and_apps(Arc::clone(&runtime), Arc::clone(&effective_config));

    debug!("Passing control to sync tasks");
    sync_main(&runtime, &effective_config).expect("Error in 'sync_main()'");
    debug!("All sync tasks ended. Waiting for Tokio tasks...");

    let tokio_result = tokio_join_handle
        .join()
        .expect("Error while joining into the Tokio runtime");

    match tokio_result {
        false => {
            debug!("All Tokio tasks ended. An error was detected!");
            warn!("DONE! (Application ended with error in one of the Tokio tasks)");
            Err(Box::from(format!("Application ended with error in one of the Tokio tasks")))
        }
        true => {
            debug!("All Tokio tasks ended. Starting graceful shutdown process!");
            warn!("DONE! (Application ended gracefully)");
            Ok(())
        }
    }


}

/// Loads default configs from ${0}.config.ron file -- creating it with defaults if it doesn't exist
fn load_configs() -> Config {
    let program_name = std::env::args().next().expect("Program name couldn't be retrieve from args");
    let config_file = format!("{}.config.ron", program_name);
    config_ops::load_or_create_default(&config_file)
        .expect("Could not load (or create) the configuration file")
}

/// Builds the initial [Runtime] object, filling it with environment info & Globals.\
/// Counters, Metrics, Reports, Controllers and even Injections will be added / updated
/// to it as soon as they are available.
fn build_runtime() -> RwLock<Runtime> {
    RwLock::new(Runtime::new(
        std::env::current_exe()
            .map_err(|err| format!("Could not get the executable file path: {}", err))
            .unwrap().to_string_lossy().to_string()
    ))
}

/// starts the Tokio runtime and all related UIs,
fn start_tokio_runtime_and_apps(runtime: Arc<RwLock<Runtime>>, config: Arc<Config>) -> JoinHandle<bool> {

    thread::spawn(move || {
        debug!("  about to start the Tokio runtime with {} worker threads...",
               if config.tokio_threads == 0 {"all available CPUs as".to_string()} else {config.tokio_threads.to_string()});
        let mut tokio_runner = tokio::runtime::Builder::new_multi_thread();
        if config.tokio_threads > 0 {
            tokio_runner.worker_threads(config.tokio_threads as usize);
        }
        let tokio_runtime = Arc::new(tokio_runner
            .thread_stack_size(4 * 1024 * 1024)     // Default for Rust's main thread is 4M; for a spawned thread (the case here), 2M; Adjust as you wish if your algorithms are heavy on recursion
            //.unhandled_panic(UnhandledPanic::ShutdownRuntime)     // TODO For upcoming Tokio versions (this one is still in unstable): shutdown if spawned tasks panic AND we're running in debug mode
            .enable_all()
            .build()
            .unwrap());
        runtime.blocking_write().tokio_runtime = Some(Arc::clone(&tokio_runtime));
        tokio_runtime
            .block_on(async {
                let runtime_for_async_main_task = Arc::clone(&runtime);
                let config_for_async_main_task = Arc::clone(&config);
                let mut async_main_task = tokio::spawn(async move {
                    debug!("    running 'async_main()'...");
                    async_main(&runtime_for_async_main_task, &config_for_async_main_task).await
                        .map_err(|err| Box::from(format!("async_main(): Aborting due to error: {}", err)))
                });
                let runtime_for_socket_server_task = Arc::clone(&runtime);
                let config_for_socket_server_task = Arc::clone(&config);
                let mut socket_server_task = tokio::spawn(async move {
                    if let ExtendedOption::Enabled(_socket_server_config) = &config_for_socket_server_task.services.socket_server {
                        debug!("    starting Socket Server service...");
                        let socket_server_config = ArcRef::from(config_for_socket_server_task)
                            .map(|config| &*config.services.socket_server);
                        let mut socket_server_handle = frontend::socket_server::SocketServer::new(socket_server_config);
                        let (processor_stream, stream_producer, stream_closer) = frontend::socket_server::sync_processors(&runtime_for_socket_server_task, &socket_server_handle).await;
                        let processor = socket_server_handle.set_processor(processor_stream, stream_producer, stream_closer);
                        let executor_join_handle = frontend::socket_server::spawn_stream_executor(processor).await;
                        let runner_closure = socket_server_handle.runner().await?;
                        Runtime::register_socket_server(&runtime_for_socket_server_task, socket_server_handle).await;
                        let (service_runner_result, stream_executor_result) = tokio::join!(runner_closure(), executor_join_handle);
                        service_runner_result.map_err(|err| format!("service runner failed: {}", err))?;
                        stream_executor_result.map_err(|err| format!("stream executor failed: {}", err))?;
                    }
                    Ok(())
                });

                let mut all_good = true;
                let mut join_and_log = |task_handle: Result<Result<(), Box<dyn std::error::Error + Sync + Send>>, tokio::task::JoinError>, task_name: &str| {
                    match task_handle {
                        Ok(join_result) => {
                            match join_result {
                                Ok(ok) => {
                                    debug!("  '{}' task ended gracefully! Result: '{:?}'", task_name, ok);
                                },
                                Err(err) => {
                                    error!("  '{}' ended with failure: {}", task_name, err);
                                    all_good = false;
                                }
                            }
                        }
                        Err(join_err) => error!("Can't start Tokio task '{}': {:?}", task_name, join_err)
                    }
                    Some(())
                };

                let mut async_main_result    = None;
                let mut socket_server_result = None;
                while async_main_result.is_none() || socket_server_result.is_none() {
                    tokio::select! {
                        result = &mut async_main_task, if async_main_result.is_none() => {
                            async_main_result = join_and_log(result, "async_main");
                        },
                        result = &mut socket_server_task, if socket_server_result.is_none() => {
                            socket_server_result = join_and_log(result, "socket service");
                        },
                    }
                }
                all_good

            })
    })
}

/// In case no UI was provided, experimentally picks one of the available
/// which don't require further parameters to run -- this, most of the times,
/// filters out Console (form it may have several commands to coose from),
/// leaving the interactive ones as options -- such as Terminal or EGui)
fn auto_select_ui(_config: &Config) -> UiOptions {
    // if std::env("DISPLAY") {
    //     AvailableFrontends::Egui
    // } else if is_tty() && config.log != Console {
    //     AvailableFrontends::Terminal
    // } else {
    UiOptions::Console(Jobs::Daemon)
    // }
}


// LOGGING
//////////
// Facade for the `slog` crate to behave just like the `log` API
// (currently we use `slog-scope` & `slog-stdlog` crates for the heavy lifting)
use config::config::LoggingOptions;
use slog_scope::GlobalLoggerGuard;
use sloggers::{Build, types::{OverflowStrategy, Severity}};
use crate::config::Jobs;


/// Keep those levels in sync with Cargo.toml's `log` crate levels defined in features.
/// Example: features = ["max_level_debug", "release_max_level_info"]
const LOG_LEVEL: Severity = if DEBUG {
    Severity::Debug
} else {
    Severity::Info
};

/// starts a global logger according to `config` specifications
/// -- the returned value should not be dropped until the program ends
fn setup_logging(config: &Config) -> GlobalLoggerGuard {
    match &config.log {
        LoggingOptions::Quiet => build_quiet_logger(),
        LoggingOptions::ToConsole => build_console_logger(),
        LoggingOptions::ToFile {file_path, rotation_size, rotations_kept, compress_rotated} => build_file_logger(&file_path, *rotation_size, *rotations_kept, *compress_rotated)
    }
}

fn build_quiet_logger() -> GlobalLoggerGuard {
    let logger = sloggers::null::NullLoggerBuilder {}
        .build()
        .expect("Could not create a 'quiet' logger");
    let log_guard = slog_scope::set_global_logger(logger);
    slog_stdlog::init().unwrap();
    log_guard
}

fn build_console_logger() -> GlobalLoggerGuard{
    let mut builder = sloggers::terminal::TerminalLoggerBuilder::new();
    builder.level(LOG_LEVEL);
    builder.destination(sloggers::terminal::Destination::Stdout);
    let logger = builder.build().expect("Could not create a 'console' logger");
    let log_guard = slog_scope::set_global_logger(logger);
    slog_stdlog::init().unwrap();
    log_guard
}

fn build_file_logger(log_file: &str, rotate_size: usize, rotate_keep: usize, rotate_compress: bool) -> GlobalLoggerGuard {
    let mut builder = sloggers::file::FileLoggerBuilder::new(log_file);
    builder.overflow_strategy(OverflowStrategy::Block);
    builder.rotate_size(rotate_size as u64);
    builder.rotate_keep(rotate_keep);
    builder.rotate_compress(rotate_compress);
    builder.level(LOG_LEVEL);
    let logger = builder.build().expect("Could not create a file logger");
    let log_guard = slog_scope::set_global_logger(logger);
    slog_stdlog::init().unwrap();
    log_guard
}
