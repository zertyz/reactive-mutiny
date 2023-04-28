//! Contains general operations on configs -- useful for the full plethora of apps using the `kickass-app-template`

use super::{config,*};
use std::fs;
use regex::Regex;
use crate::LoggingOptions;


/// Returns the result of merging the given `low_priority` and `high_priority` configs,
/// ensuring they adhere to the config contract required by the business logic modules,
/// filling in (with defaults) any missing pieces.\
/// --> Most probably, `low_priority` comes from the config file while `high_priority`
/// comes from the command line arguments (see [command_line::config_from_command_line_options])
/// NOTE: may panic if the resulting config does adhere to the contract
pub fn merge_configs(mut low_priority: Config, mut high_priority: Config) -> Config {
    // shoves low_priority into any missing pieces of high_priority and returns it

    // KICKASS APP TEMPLATE
    ///////////////////////

    // case: file logging is partially specified in the high priority -- pieces of the low priority (or default values) fills in
    if let LoggingOptions::ToFile { file_path: ref _file_path, rotation_size: mut _rotation_size, rotations_kept: mut _rotations_kept, compress_rotated: mut _compress_rotated } = high_priority.log {
        if _rotation_size == 0 {
            if let LoggingOptions::ToFile { file_path: _l_file_path, rotation_size: l_rotation_size, rotations_kept: l_rotations_kept, compress_rotated: l_compress_rotated } = low_priority.log {
                _rotation_size    = l_rotation_size;
                _rotations_kept   = l_rotations_kept;
                _compress_rotated = l_compress_rotated;
            } else {
                _rotation_size    = 1024*1024*1024;
                _rotations_kept   = 64;
                _compress_rotated = true;
            }
        }
    }

    // TODO: case fix: command-line always specifies a UI... so there is no point in having it into the config file
    //high_priority.ui = high_priority.ui;

    // sets services in both low & high_priority -- so merging the following cases gets simpler
    if !high_priority.services.is_enabled() {
        high_priority.services = ExtendedOption::Enabled(ServicesConfig {
            web:           ExtendedOption::Unset,
            socket_server: ExtendedOption::Unset,
            telegram:      ExtendedOption::Unset
        });
    }
    if !low_priority.services.is_enabled() {
        low_priority.services = ExtendedOption::Enabled(ServicesConfig {
            web:           ExtendedOption::Unset,
            socket_server: ExtendedOption::Unset,
            telegram:      ExtendedOption::Unset
        });
    }

    // case: Telegram service is, currently, only definable in the `low_priority`
    if let ExtendedOption::Enabled(l_telegram) = &low_priority.services.telegram {
        high_priority.services.telegram = ExtendedOption::Enabled(l_telegram.clone());
    }

    // case: Rocket service is, currently, only definable in the `low_priority`
    if let ExtendedOption::Enabled(l_web) = &low_priority.services.web {
        high_priority.services.web = ExtendedOption::Enabled(l_web.clone());
    }

    // case: Socket server is, currently, only definable in the `low_priority`
    if let ExtendedOption::Enabled(l_socket_server) = &low_priority.services.socket_server {
        high_priority.services.socket_server = ExtendedOption::Enabled(l_socket_server.clone());
    }

    // case: tokio_threads: defaults to 0 -- considered as unset if < 0
    high_priority.tokio_threads = if high_priority.tokio_threads > 0 {
        high_priority.tokio_threads
    } else if low_priority.tokio_threads > 0 {
        low_priority.tokio_threads
    } else {
        0
    };

    // APP's merges goes here
    /////////////////////////

    // suggested behavior if conflicts arise: panic!("merge_configs: no viable 'XXXXXXXXXX' was provided. Execution aborted. HINT: config is likely invalid -- backup & delete the config file to have the default value rewritten");

    high_priority
}

/// loads the application-wide configuration from the given `config_file_path`
/// or create it (with default values) if it doesn't exist
pub fn load_or_create_default(config_file_path: &str) -> Result<Config, Box<dyn std::error::Error>> {
    let result = load_from_file(config_file_path);
    if result.is_ok() {
        result
    } else {
        let error_message = result.as_ref().unwrap_err().to_string();
        let default_config = Config::default();
        if error_message.contains("No such file or directory") {
            save_to_file(&default_config, config_file_path)?;
            Ok(default_config)
        } else {
            result
        }
    }
}

/// loads the application-wide configuration from the given `config_file_path`, if possible
fn load_from_file(config_file_path: &str) -> Result<Config, Box<dyn std::error::Error>> {
    let ron_file_contents = fs::read_to_string(config_file_path)?;
    let ron_options = ron::Options::default()
        .with_default_extension(ron_extensions());
    ron_options.from_str(&ron_file_contents)
        .map_err(|err| Box::from(format!("config_ops.rs: Error deserializing contents of file '{}' as RON: {} -- HINT: delete the config file and let it be regenerated with all the default options", config_file_path, err)))
}

/// transcription of the config model, for documentation purposes when writing the default config file
const CONFIG_MODELS_DOCS: &str = include_str!("config.rs");

/// saves the application-wide `config` to `config_file_path`,
/// including documentation from the original [config_model] sources
fn save_to_file(config: &Config, config_file_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let data_section = ron::ser::to_string_pretty(
        &config,
        ron::ser::PrettyConfig::new()
            .depth_limit(10)
            .new_line(String::from("\n"))
            .indentor(String::from("    "))
            .separate_tuple_members(true)
            .enumerate_arrays(true)
            //.decimal_floats(true)
            .extensions(ron_extensions()))
        .map_err(|err| format!("config.rs: Error serializing config as TOML: {}", err))?;

    // include documentation on the written file, with the Regex replacements declared there
    let docs_section = config::REPLACEMENTS.iter()
        .fold(String::from(CONFIG_MODELS_DOCS), |s, (from, to)| {
            let regex = Regex::new(from).expect("Error parsing regex");
            regex.replace_all(&s, *to).to_string()
        });

    let config_file_contents = format!("{}\n\n/*{}*/\n", data_section, docs_section);

    fs::write(config_file_path, config_file_contents)
        .map_err(|err| Box::from(format!("config_ops.rs: Error writing default RON config to file '{}': {}", config_file_path, err)))
}

/// builds & returns the RON extensions used to load and save our .ron files
fn ron_extensions() -> ron::extensions::Extensions {
    let mut extensions = ron::extensions::Extensions::empty();
    extensions.insert(ron::extensions::Extensions::IMPLICIT_SOME);
    extensions.insert(ron::extensions::Extensions::UNWRAP_NEWTYPES);
    // as of 2022-07-22, ron 0.7.1, using this extensions gives us bugs (written file cannot be loaded)
//    extensions.insert(ron::extensions::Extensions::UNWRAP_VARIANT_NEWTYPES);
    extensions
}

/// Unit tests the [config](self) module
#[cfg(any(test,doc))]
mod tests {
    use super::*;

    const TEST_CONFIG_FILE: &str = "/tmp/kickass-app-template-tests.config.ron";

    #[cfg_attr(not(doc),test)]
    fn file_load_and_save() {
        fs::remove_file(TEST_CONFIG_FILE).unwrap_or(());

        // load non existing file
        let result = load_from_file(TEST_CONFIG_FILE);
        println!("Loading from inexisting file: gives '{:?}'", result);
        assert!(result.is_err(), "Loading from an non existing file should have returned an error");
        let error_message = result.unwrap_err().to_string();
        assert!(error_message.contains("No such file or directory"), "Error message '{}' does not contain 'No such file or directory'", error_message);

        // save to non existing file
        save_to_file(&Config::default(), TEST_CONFIG_FILE)
            .expect("Could not save config file");

        // load the -- now existing -- file
        let _result = load_from_file(TEST_CONFIG_FILE)
            .expect("Could not load config file just created by save. Does RON have a bug or its default saving & loading parameters needs tweaking?");

        // check load_or_create_default() for existing file
        let _result = load_or_create_default(TEST_CONFIG_FILE)
            .expect("Could not load_or_create_default() for an existing file");

        fs::remove_file(TEST_CONFIG_FILE).unwrap_or(());

        // check load_or_create_default() for non existing file
        let _result = load_or_create_default(TEST_CONFIG_FILE)
            .expect("Could not load_or_create_default() for a non existing file");
    }

    /// assures [merge_configs()] addresses all cases
    #[cfg_attr(not(doc),test)]
    fn merging_completenes() {

        // checks high priority is honored
        let low = Config {
            log:           LoggingOptions::Quiet,
            services:      ExtendedOption::Unset,
            tokio_threads: 0,
            ui:            ExtendedOption::Unset,

        };
        let high = Config::default();
        let expected = Config::default();
        let merged = merge_configs(low, high);
        assert_eq!(merged, expected, "'merge_configs() seem to not be covering newly added configs well: High priority config got (wrongly?) overridden by low priority");

        // checks low priority has its voice
        let low = Config::default();
        let high = Config {
            log:           LoggingOptions::ToConsole,
            services:      ExtendedOption::Unset,
            tokio_threads: 0,
            ui:            ExtendedOption::Unset,

        };
        let expected = Config::default();
        let merged = merge_configs(low, high);
        assert_eq!(merged, expected, "'merge_configs() seem to not be covering newly added configs well: Low priority config wasn't able to set unset properties in the high priority");

    }

}