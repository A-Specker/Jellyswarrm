use serde::{Deserialize, Serialize};
use url::Url;

use crate::encryption::Password;

/// One `[[plugins]]` entry in `jellyswarrm.toml`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PluginConfig {
    /// Unique name, used in URLs: lowercase ASCII letters, digits and `-`.
    pub name: String,
    /// Base URL of the plugin service, as reachable from Jellyswarrm.
    pub url: Url,
    /// Shared secret for both directions. `Password` keeps it out of logs.
    pub token: Password,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_enabled() -> bool {
    true
}

pub fn is_valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_entry_and_defaults_to_enabled() {
        let config: PluginConfig = toml::from_str(
            r#"
            name = "viewer"
            url = "http://viewer:8080"
            token = "secret"
            "#,
        )
        .unwrap();

        assert_eq!(config.name, "viewer");
        assert_eq!(config.url.as_str(), "http://viewer:8080/");
        assert!(config.enabled);
    }

    #[test]
    fn debug_output_hides_token() {
        let config = PluginConfig {
            name: "viewer".to_string(),
            url: Url::parse("http://viewer:8080").unwrap(),
            token: Password::from("secret"),
            enabled: true,
        };

        assert!(!format!("{config:?}").contains("secret"));
    }

    #[test]
    fn validates_names() {
        assert!(is_valid_name("viewer"));
        assert!(is_valid_name("now-playing-2"));
        assert!(!is_valid_name(""));
        assert!(!is_valid_name("Viewer"));
        assert!(!is_valid_name("../admin"));
        assert!(!is_valid_name("now playing"));
    }
}
