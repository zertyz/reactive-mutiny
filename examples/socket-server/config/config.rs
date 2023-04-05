
//! Keep, in this module, all the types used for the applications config file
//! -- the file will be placed along the `${0}.config.ron` config file to
//! serve as it's documentation

use std::ops::{Deref, DerefMut};
use serde::{Serialize, Deserialize};
use structopt::{StructOpt};


/// CONFIG FILE DOCUMENTATION
/// (feel free to move comments & possible values close to the data)
///
/// Root for this Application's config
#[derive(Debug,PartialEq,Serialize,Deserialize)]
pub struct Config {

    // kickass-app-template
    ///////////////////////

    /// Specifies what the application should do with it's log messages
    pub log: LoggingOptions,
    /// Services (and their configs) to be enabled
    pub services: ExtendedOption<ServicesConfig>,
    /// The number of threads to dedicate to Tokio -- if not 1, make it no greater than the number of CPUs,
    /// unless you (wrongly) are waiting on Tokio threads.
    /// Set it to 0 to use all available CPUs the process has access to
    pub tokio_threads: i16,

    // business logic
    /////////////////

    // (configs related to the app logic go here)

    /// The UI that should be used to run the application
    pub ui: ExtendedOption<UiOptions>,
}

/// UI options -- how the application will interact with users
#[derive(Debug,PartialEq,Clone,Copy,Serialize,Deserialize,StructOpt)]
pub enum UiOptions {
    // /// Collects running environment data to determine the best possible Ui to use:
    // /// if DISPLAY env is available, `Egui` is used, otherwise use `Terminal` if
    // /// the appropriate TERM env is defined. `Console` is used as a fallback.
    // Automatic,
    /// Runs the application's console UI -- run `${0} console --help` for more details
    Console(Jobs),
}

#[derive(Debug,PartialEq,Clone,Serialize,Deserialize)]
pub struct ServicesConfig {
    pub web:           ExtendedOption<WebConfig>,
    pub socket_server: ExtendedOption<SocketServerConfig>,
    pub telegram:      ExtendedOption<TelegramConfig>,
}

/// The telegram service
#[derive(Debug,PartialEq,Clone,Serialize,Deserialize)]
pub struct TelegramConfig {
    /// Telegram's bot token, obtained from "BotFather's" bot:
    /// 1) Open TelegramApp and search for BotFather
    /// 2) Send /newbot (or /help)
    pub token: String,
    /// The bot to use
    pub bot: TelegramBotOptions,
    /// chat ids where send notifications will land on
    pub notification_chat_ids: Vec<i64>,
}

/// Available bots to handle Telegram interaction
#[derive(Debug,PartialEq,Clone,Serialize,Deserialize)]
pub enum TelegramBotOptions {
    /// Simply answers each message with a dice throw
    Dice,
    /// Only answers to known commands. Initiate a chat with this bot by sending "/help"
    Stateless,
    /// Chat-like robot, holding dialog context. Send it anything to start the conversations
    Stateful,
}

/// Rocket requires us to inform in which "environment" we're running.\
/// If you use the [RocketConfigOptions::StandardRocketTomlFile] variant, each section
/// must be present on the file.
#[derive(Debug,PartialEq,Clone,Serialize,Deserialize)]
pub enum RocketProfiles {
    Debug,
    Production,
}

/// Available Rocket configuration possibilities
#[derive(Debug,PartialEq,Clone,Serialize,Deserialize)]
pub enum RocketConfigOptions {
    /// Instructs Rocket to read configs from it's `Rocket.toml` file. Notice that Rocket will look
    /// for such file in the current working directory, rather than on the executable's location.
    StandardRocketTomlFile,
    /// When using use only HTTP, using this variant may be desireable, as it avoids the need of managing
    /// another configuration file: `Rocket.toml` -- Rocket's config.
    Provided {
        /// Port to listen to HTTP connections
        http_port:  u16,
        /// How many tokio async tasks should be used to process the incoming requests?
        workers: u16,
    }
}

/// The HTTP/HTTPS service
#[derive(Debug,PartialEq,Clone,Serialize,Deserialize)]
pub struct WebConfig {
    /// The Rocket profile to use as basis for `rocket_config`
    pub profile: RocketProfiles,
    /// Rocket config details
    pub rocket_config: RocketConfigOptions,
    /// If set, enables [crate::frontend::web::sanity_check] routes -- allowing this executable to be probed for it's running sanity
    pub sanity_check_routes: bool,
    /// If set, enables [crates::frontend::web::stats] routes -- exposing runtime metrics
    pub stats_routes: bool,
    /// If set, enables [crates::frontend::web::logs_following] routes -- exposing online logs for the app
    pub logs_following_routes: bool,
    /// If set, enables [crates::frontend::web::ogre_events_following] routes -- exposing online `Ogre Events` for the app
    pub ogre_events_following_routes: bool,
    /// If set, enables [crates::frontend::web::ogre_events_queue] routes -- exposing `Ogre Events` designed to be consumed by external services
    pub ogre_events_queue_routes: bool,
    /// If set, enables the Angular application present in `web-app/`, exposing it's [crate::frontend::web::backend]
    /// routes and all related static files (see [crate::frontend::web::embedded_files])
    pub web_app: bool,
    /// Prepends the given string to all our HTTP/HTTPS routes
    pub routes_prefix: String,
}

/// The socket server
#[derive(Debug,PartialEq,Clone,Serialize,Deserialize)]
pub struct SocketServerConfig {
    /// the interface's IP to listen to -- 0.0.0.0 will cause listening to all network interfaces
    pub interface: String,
    /// what port to listen to
    pub port:      u16,
    /// How many tokio async tasks should be used to process the incoming requests?
    /// If you delegate it to events (or similar), this should be 1;
    /// If you fully process the request in the worker task (bad practice), measure and pick your optimal number.
    pub workers: u16,
}

/// Logging options -- what to do with log messages
#[derive(Debug,PartialEq,Serialize,Deserialize)]
pub enum LoggingOptions {
    /// Simply ignore them
    Quiet,
    /// Output them to stdout
    ToConsole,
    /// Save them to the specified file, with the specified options:
    ToFile {
        /// File to use a basis for rotation or appending
        file_path: String,
        /// The maximum size (bytes) for a log file before a rotation kicks in -- example: 1024*1024*1024 = 1073741824
        rotation_size: usize,
        /// The upper limit of rotations to keep before deleting old ones -- example: 64
        rotations_kept: usize,
        /// Performs a gzip compression after a rotation?
        compress_rotated: bool,
    },
}

/////  EVERYTHING BELOW THIS LINE WILL NOT BE INCLUDED IN THE APPLICATION'S CONFIG FILE  /////

/// Jobs that this application supports. Maps to the command line options [crate::command_line::Jobs]
#[derive(Debug,PartialEq,Clone,Copy,Serialize,Deserialize,StructOpt)]
pub enum Jobs {
    /// Long-Runner: Starts the service, only quitting when a SIG_TERM is received
    Daemon,
    /// Inspects & shows the effective configs & runtime used by the application, then quits
    CheckConfig,
    // ...
}

/// A simple extension to the default `Option` to allow distinction for the None state (is it unset or forcibly disabled?)
#[derive(Debug,PartialEq,Clone,Serialize,Deserialize)]
pub enum ExtendedOption<T> {
    Unset,
    Disabled,
    Enabled(T),
}
impl<T> ExtendedOption<T> {
    pub fn is_enabled(&self) -> bool {
        if let ExtendedOption::Enabled(_) = self {
            true
        } else {
            false
        }
    }
}
impl<T> Deref for ExtendedOption<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        match self {
            ExtendedOption::Enabled(raw) => raw,
            ExtendedOption::Unset             => panic!("BUG! attempted to `deref` the (non-existing) raw value -- from an 'Unset' variant of 'ExtendedOption'"),
            ExtendedOption::Disabled          => panic!("BUG! attempted to `deref` the (non-existing) raw value -- from an 'Disabled' variant of 'ExtendedOption'"),
        }
    }
}
impl<T> DerefMut for ExtendedOption<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        match self {
            ExtendedOption::Enabled(raw) => raw,
            ExtendedOption::Unset                => panic!("BUG! attempted to `deref_mut` the (non-existing) raw value -- from an 'Unset' variant of 'ExtendedOption'"),
            ExtendedOption::Disabled             => panic!("BUG! attempted to `deref_mut` the (non-existing) raw value -- from an 'Disabled' variant of 'ExtendedOption'"),
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            log:           LoggingOptions::ToConsole,
            services:      ExtendedOption::Enabled(
                               ServicesConfig {
                                   telegram: ExtendedOption::Enabled(TelegramConfig {
                                           token: String::from("<<Open TelegramApp, search for BotFather, send /newbot>>"),
                                           bot:   TelegramBotOptions::Stateless,
                                           notification_chat_ids: vec![
                                               9999999999,    // james smith
                                               9999999999,    // mary johnson
                                           ],
                                       }),
                                   web: ExtendedOption::Enabled(WebConfig {
                                       profile: RocketProfiles::Debug,
                                       rocket_config: RocketConfigOptions::Provided {
                                           http_port: 8000,
                                           workers:   1,
                                       },
                                       sanity_check_routes:          false,
                                       stats_routes:                 false,
                                       logs_following_routes:        false,
                                       ogre_events_following_routes: false,
                                       ogre_events_queue_routes:     false,
                                       web_app:                      true,
                                       routes_prefix: "".to_string()
                                   }),
                                   socket_server: ExtendedOption::Enabled(SocketServerConfig {
                                       interface: "0.0.0.0".to_string(),
                                       port: 9758,
                                       workers: 1,
                                   }),
                               }
                           ),
            tokio_threads: 0,
            ui:            ExtendedOption::Enabled(UiOptions::Console(Jobs::Daemon)),
        }
    }
}

/// Regexes and their replacements to apply to this file when writing the docs
pub const REPLACEMENTS: &[(&str, &str)] = &[
    ("\n//![^\n]*",                                                                                            ""),     // remove file doc comments
    ("\nuse serde::[^\n]*",                                                                                    ""),     // remove 'use' clause
    ("\n#[^\n]*",                                                                                              ""),     // remove macros & #[derive(...)] clauses
    ("(?s)\n/////  EVERYTHING BELOW THIS LINE WILL NOT BE INCLUDED IN THE APPLICATION'S CONFIG FILE  /////.*", ""),     // remove everything after the comment tag
    ("\n\n+",                                                                                                  "\n\n"), // standardize the number of consecutive empty lines
];

