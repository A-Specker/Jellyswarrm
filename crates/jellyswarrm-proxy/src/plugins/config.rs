use serde::{de::Error as _, Deserialize, Deserializer, Serialize};
use url::Url;

use crate::encryption::Password;

/// One `[[plugins]]` entry in `jellyswarrm.toml`, or one element of the
/// `JELLYSWARRM_PLUGINS` JSON list.
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

/// `plugins` comes as a list from `[[plugins]]` in the config file, or as a JSON
/// string from the `JELLYSWARRM_PLUGINS` environment variable, which can't hold a
/// list (e.g. in Docker Compose).
pub fn deserialize_plugins<'de, D>(deserializer: D) -> Result<Vec<PluginConfig>, D::Error>
where
    D: Deserializer<'de>,
{
    match serde_json::Value::deserialize(deserializer)? {
        serde_json::Value::String(json) if json.trim().is_empty() => Ok(Vec::new()),
        serde_json::Value::String(json) => serde_json::from_str(&json).map_err(|e| {
            D::Error::custom(format!(
                "JELLYSWARRM_PLUGINS must be a JSON list of plugins \
                 like [{{\"name\":\"viewer\",\"url\":\"http://viewer:8765\",\"token\":\"...\"}}]: {e}"
            ))
        }),
        list => serde_json::from_value(list).map_err(D::Error::custom),
    }
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

    /// Load `AppConfig` the way `config::load_config` does: file first, then the
    /// environment (injected here instead of read from the process).
    fn load(toml: &str, env: &[(&str, &str)]) -> Result<crate::config::AppConfig, String> {
        config::Config::builder()
            .add_source(config::File::from_str(toml, config::FileFormat::Toml))
            .add_source(
                config::Environment::with_prefix("JELLYSWARRM")
                    .separator("_")
                    .source(Some(
                        env.iter()
                            .map(|(k, v)| (k.to_string(), v.to_string()))
                            .collect(),
                    )),
            )
            .build()
            .and_then(|source| source.try_deserialize())
            .map_err(|e| e.to_string())
    }

    const TOML_PLUGIN: &str = r#"
        [[plugins]]
        name = "from-file"
        url = "http://file:8765"
        token = "file-token"
    "#;

    #[test]
    fn reads_plugins_from_config_file() {
        let config = load(TOML_PLUGIN, &[]).unwrap();

        assert_eq!(config.plugins.len(), 1);
        assert_eq!(config.plugins[0].name, "from-file");
    }

    #[test]
    fn reads_plugins_from_environment_json() {
        let config = load(
            "",
            &[(
                "JELLYSWARRM_PLUGINS",
                r#"[{"name":"viewer","url":"http://viewer:8765","token":"env-token"},
                    {"name":"off","url":"http://off:1","token":"t","enabled":false}]"#,
            )],
        )
        .unwrap();

        let names: Vec<_> = config.plugins.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["viewer", "off"]);
        assert_eq!(config.plugins[0].token.as_str(), "env-token");
        assert!(config.plugins[0].enabled);
        assert!(!config.plugins[1].enabled);
    }

    #[test]
    fn environment_replaces_config_file_list() {
        let config = load(
            TOML_PLUGIN,
            &[(
                "JELLYSWARRM_PLUGINS",
                r#"[{"name":"viewer","url":"http://viewer:8765","token":"t"}]"#,
            )],
        )
        .unwrap();

        let names: Vec<_> = config.plugins.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["viewer"]);
    }

    #[test]
    fn empty_environment_value_means_no_plugins() {
        assert!(load("", &[("JELLYSWARRM_PLUGINS", "  ")])
            .unwrap()
            .plugins
            .is_empty());
        assert!(load("", &[]).unwrap().plugins.is_empty());
    }

    #[test]
    fn invalid_environment_json_names_the_variable() {
        for value in [
            "viewer",
            r#"{"name":"viewer"}"#,
            r#"[{"name":"viewer","url":"not a url","token":"t"}]"#,
        ] {
            let error = load("", &[("JELLYSWARRM_PLUGINS", value)]).unwrap_err();
            assert!(error.contains("JELLYSWARRM_PLUGINS"), "{value}: {error}");
        }
    }

    #[test]
    fn invalid_config_file_entry_keeps_a_useful_error() {
        let error = load(
            r#"
            [[plugins]]
            name = "viewer"
            url = "http://viewer:8765"
            "#,
            &[],
        )
        .unwrap_err();

        assert!(error.contains("token"), "{error}");
    }

    #[test]
    fn plugins_from_environment_are_saved_as_a_toml_list() {
        let config = load(
            "",
            &[(
                "JELLYSWARRM_PLUGINS",
                r#"[{"name":"viewer","url":"http://viewer:8765","token":"t"}]"#,
            )],
        )
        .unwrap();

        let saved = toml::to_string_pretty(&config).unwrap();
        assert!(saved.contains("[[plugins]]"), "{saved}");
        assert_eq!(load(&saved, &[]).unwrap().plugins.len(), 1);
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
