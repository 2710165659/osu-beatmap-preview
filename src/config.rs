//! Compile-time application configuration.
//!
//! `build.rs` reads `assets/default_config.yml` and emits typed Rust constants
//! into the build directory. Keeping the source YAML embedded here makes the
//! resource part of the binary and gives consumers a single authoritative
//! configuration file.

#[allow(dead_code)]
pub fn default_config_yaml() -> &'static str {
    include_str!("../assets/default_config.yml")
}

include!(concat!(env!("OUT_DIR"), "/config_constants.rs"));
