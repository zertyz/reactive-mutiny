//! See [super]

use crate::config::*;
use structopt::{StructOpt};


pub fn parse_from_args() -> CommandLineOptions {
    CommandLineOptions::from_args()
}

/// merges the higher priority command line options with the application-wide config (which, most probably, came from parsing the configuration file),
/// returning a new, merged, application-wide config or panicking, if there are inconsistencies
pub fn merge_config_file_and_command_line_options(app_config_from_file: Config, command_line_options: CommandLineOptions) -> Config {
    if DEBUG {
        println!("'{}' Command Line options: {:#?}", APP_NAME, command_line_options);
        println!("'{}' Config file options: {:#?}", APP_NAME, app_config_from_file);
    }
    let app_config_from_command_line = config_from_command_line_options(&command_line_options);
    let effective_config = config_ops::merge_configs(app_config_from_file, app_config_from_command_line);
    if DEBUG {
        println!("'{}' Effective config: {:#?}", APP_NAME, effective_config);
    }
    effective_config
}

/// Command-line options
#[derive(Debug,StructOpt)]
#[structopt(about = "
================================================================
Here you should add a brief description of what this executable
does. Be as succinct as possible, but no more succinct than that.
Default & advanced options are in ${0}.config. Some of them may
be overridden by the command-line options bellow:
================================================================
")]
pub struct CommandLineOptions {


    // KICKASS Flags & Options
    //////////////////////////

    /// Suppresses all output to stdout and stderr
    #[structopt(long)]
    quiet: bool,

    /// Sends all logs to the given file
    #[structopt(long)]
    log_to_file: Option<String>,

    /// Which UI to use to run the application
    #[structopt(subcommand)]
    pub runner: UiOptions,


    // LOGIC options
    ////////////////
    // here goes the first level of your program's command line options

    // ...

}

/// Creates the application-wide `Config` out of the command-line options
/// -- even if the config is incomplete.
fn config_from_command_line_options(command_line_options: &CommandLineOptions) -> Config {
    Config {
        log: if let Some(file_path) = &command_line_options.log_to_file {
                 LoggingOptions::ToFile {
                     file_path:        file_path.to_string(),
                     rotation_size:    0,
                     rotations_kept:   0,
                     compress_rotated: false,
                 }
             } else if command_line_options.quiet {
                 LoggingOptions::Quiet
             } else {
                 LoggingOptions::ToConsole
             },
        services: ExtendedOption::Unset,
        tokio_threads: -1,
        ui: ExtendedOption::Enabled(command_line_options.runner),
    }
}
