//! Compile-time application configuration.
//!
//! `build.rs` reads `assets/default_config.yml` and emits typed Rust constants
//! into the build directory. Keeping the source YAML embedded here makes the
//! resource part of the binary and gives consumers a single authoritative
//! configuration file.

use std::path::PathBuf;

#[allow(dead_code)]
pub fn default_config_yaml() -> &'static str {
    include_str!("../assets/default_config.yml")
}

include!(concat!(env!("OUT_DIR"), "/config_constants.rs"));

/// Expand portable directory placeholders from the embedded configuration.
/// `%TEMP%` always uses the platform's temporary directory; other `%NAME%`
/// placeholders are resolved from the process environment when available.
pub(crate) fn resolve_path(template: &str) -> PathBuf {
    let mut expanded = String::with_capacity(template.len());
    let mut remainder = template;
    while let Some(start) = remainder.find('%') {
        expanded.push_str(&remainder[..start]);
        let after_start = &remainder[start + 1..];
        let Some(end) = after_start.find('%') else {
            expanded.push_str(&remainder[start..]);
            remainder = "";
            break;
        };
        let name = &after_start[..end];
        let value = if name.eq_ignore_ascii_case("TEMP") {
            Some(std::env::temp_dir().to_string_lossy().into_owned())
        } else {
            std::env::var_os(name).map(|value| value.to_string_lossy().into_owned())
        };
        match value {
            Some(value) => expanded.push_str(&value),
            None => {
                expanded.push('%');
                expanded.push_str(name);
                expanded.push('%');
            }
        }
        remainder = &after_start[end + 1..];
    }
    expanded.push_str(remainder);
    PathBuf::from(expanded)
}

#[cfg(test)]
mod tests {
    use super::resolve_path;

    #[test]
    fn expands_temp_placeholder() {
        let path = resolve_path("%TEMP%/osu-beatmap-preview");
        assert_eq!(path, std::env::temp_dir().join("osu-beatmap-preview"));
    }
}
