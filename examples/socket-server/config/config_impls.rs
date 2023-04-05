//! Contains implementations (of business rules) on any models declared in `config_models.rs`

use super::*;


impl Config {

    /// returns true whether we're both logging to console and our queries were set to output to console as well
    /// -- in this case, special care should be taken so that log messages don't get mangled with the output
    /// (for instance, waits must be set)
    pub fn is_console_output_shared(&self) -> bool {
        if let LoggingOptions::ToConsole = self.log {
            self.services.telegram.is_enabled() ||
            self.services.web.is_enabled() /*||
            self.ogre_workers.is_enabled()*/
        } else {
            false
        }
    }
}
