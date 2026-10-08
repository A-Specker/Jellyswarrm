use std::{collections::HashSet, time::Duration};

use serde::Deserialize;
use tokio::sync::{broadcast, RwLock};
use tracing::{info, warn};

use super::{
    config::{is_valid_name, PluginConfig},
    events::PluginEvent,
};
use crate::url_helper::join_server_url;

/// Plugin API major version this build supports (`api_version` in the manifest).
pub const API_VERSION: u32 = 1;

/// Bounds each manifest request so an unreachable plugin cannot stall a reload.
const MANIFEST_TIMEOUT: Duration = Duration::from_secs(5);

/// Events buffered per subscriber before a slow plugin starts missing events.
const EVENT_BUFFER: usize = 256;

/// `GET {url}/manifest.json`, see `docs/plugins/plugin-api-v1.md`.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Manifest {
    pub name: String,
    pub version: String,
    pub api_version: u32,
    #[serde(default)]
    pub ui: Option<ManifestUi>,
}

// Read by the admin UI integration (roadmap step 4).
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct ManifestUi {
    pub title: String,
    #[serde(default)]
    pub icon: Option<String>,
    pub entry: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PluginStatus {
    Ok,
    Disabled,
    /// The manifest request failed (connection, timeout or HTTP error status).
    Unreachable(String),
    /// The plugin targets a plugin API version this build does not support.
    Incompatible {
        api_version: u32,
    },
    /// The configuration or the manifest is invalid.
    Invalid(String),
}

#[derive(Debug, Clone)]
pub struct Plugin {
    pub config: PluginConfig,
    pub manifest: Option<Manifest>,
    pub status: PluginStatus,
}

/// Tracks the configured plugins and their manifests. Plugins never get access
/// to `AppState`; later steps expose data to them through this manager.
pub struct PluginManager {
    client: reqwest::Client,
    plugins: RwLock<Vec<Plugin>>,
    events: broadcast::Sender<PluginEvent>,
}

impl PluginManager {
    pub fn new(client: reqwest::Client) -> Self {
        Self {
            client,
            plugins: RwLock::new(Vec::new()),
            events: broadcast::channel(EVENT_BUFFER).0,
        }
    }

    /// Send `event` to all subscribed plugins. Never blocks; without subscribers
    /// the event is dropped.
    pub fn publish(&self, event: PluginEvent) {
        let _ = self.events.send(event);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<PluginEvent> {
        self.events.subscribe()
    }

    pub fn has_subscribers(&self) -> bool {
        self.events.receiver_count() > 0
    }

    /// Replace the plugin list with `configs`, fetching all manifests concurrently.
    pub async fn load(&self, configs: &[PluginConfig]) {
        let mut seen = HashSet::new();
        let checks = configs.iter().map(|config| {
            let duplicate = !seen.insert(config.name.as_str());
            self.check(config, duplicate)
        });
        let plugins = futures_util::future::join_all(checks).await;

        for plugin in &plugins {
            log_status(plugin);
        }
        *self.plugins.write().await = plugins;
    }

    // Read by the plugin API and admin UI (roadmap steps 2 and 4).
    #[allow(dead_code)]
    pub async fn plugins(&self) -> Vec<Plugin> {
        self.plugins.read().await.clone()
    }

    /// Name of the plugin owning `token`, if that plugin may use the plugin API.
    /// Unreachable plugins are accepted: a plugin container may start after us.
    pub async fn authenticate(&self, token: &str) -> Option<String> {
        self.plugins
            .read()
            .await
            .iter()
            .find(|plugin| {
                matches!(
                    plugin.status,
                    PluginStatus::Ok | PluginStatus::Unreachable(_)
                ) && constant_time_eq(plugin.config.token.as_str(), token)
            })
            .map(|plugin| plugin.config.name.clone())
    }

    async fn check(&self, config: &PluginConfig, duplicate: bool) -> Plugin {
        let (manifest, status) = if duplicate {
            (
                None,
                PluginStatus::Invalid("duplicate plugin name".to_string()),
            )
        } else if !is_valid_name(&config.name) {
            (
                None,
                PluginStatus::Invalid(
                    "name may only contain lowercase letters, digits and '-'".to_string(),
                ),
            )
        } else if !config.enabled {
            (None, PluginStatus::Disabled)
        } else {
            match self.fetch_manifest(config).await {
                Ok(manifest) => {
                    let status = validate(config, &manifest);
                    (Some(manifest), status)
                }
                Err(status) => (None, status),
            }
        };

        Plugin {
            config: config.clone(),
            manifest,
            status,
        }
    }

    async fn fetch_manifest(&self, config: &PluginConfig) -> Result<Manifest, PluginStatus> {
        let url = join_server_url(&config.url, "/manifest.json");
        let response = self
            .client
            .get(url)
            .bearer_auth(config.token.as_str())
            .timeout(MANIFEST_TIMEOUT)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|e| PluginStatus::Unreachable(e.to_string()))?;

        response
            .json::<Manifest>()
            .await
            .map_err(|e| PluginStatus::Invalid(format!("invalid manifest: {e}")))
    }
}

/// Compare secrets without leaking the position of the first mismatch through timing.
fn constant_time_eq(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |diff, (x, y)| diff | (x ^ y))
            == 0
}

fn validate(config: &PluginConfig, manifest: &Manifest) -> PluginStatus {
    if manifest.name != config.name {
        PluginStatus::Invalid(format!(
            "manifest name '{}' does not match configured name",
            manifest.name
        ))
    } else if manifest.api_version != API_VERSION {
        PluginStatus::Incompatible {
            api_version: manifest.api_version,
        }
    } else {
        PluginStatus::Ok
    }
}

fn log_status(plugin: &Plugin) {
    let name = &plugin.config.name;
    match &plugin.status {
        PluginStatus::Ok => {
            let version = plugin.manifest.as_ref().map_or("?", |m| m.version.as_str());
            info!("Plugin '{name}' {version} loaded from {}", plugin.config.url);
        }
        PluginStatus::Disabled => info!("Plugin '{name}' is disabled"),
        PluginStatus::Unreachable(error) => {
            warn!("Plugin '{name}' is unreachable: {error}")
        }
        PluginStatus::Incompatible { api_version } => warn!(
            "Plugin '{name}' requires plugin API v{api_version}, this build supports v{API_VERSION}; disabled"
        ),
        PluginStatus::Invalid(reason) => warn!("Plugin '{name}' is invalid: {reason}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encryption::Password;
    use url::Url;
    use wiremock::{
        matchers::{header, method, path},
        Mock, MockServer, ResponseTemplate,
    };

    fn plugin_config(name: &str, url: &str) -> PluginConfig {
        PluginConfig {
            name: name.to_string(),
            url: Url::parse(url).unwrap(),
            token: Password::from("secret"),
            enabled: true,
        }
    }

    async fn serve_manifest(server: &MockServer, manifest: serde_json::Value) {
        Mock::given(method("GET"))
            .and(path("/manifest.json"))
            .and(header("authorization", "Bearer secret"))
            .respond_with(ResponseTemplate::new(200).set_body_json(manifest))
            .mount(server)
            .await;
    }

    async fn load_one(config: PluginConfig) -> Plugin {
        let manager = PluginManager::new(reqwest::Client::new());
        manager.load(&[config]).await;
        manager.plugins().await.remove(0)
    }

    #[tokio::test]
    async fn loads_manifest_with_token() {
        let server = MockServer::start().await;
        serve_manifest(
            &server,
            serde_json::json!({
                "name": "viewer",
                "version": "0.1.0",
                "api_version": 1,
                "ui": { "title": "Now playing", "entry": "index.html" }
            }),
        )
        .await;

        let plugin = load_one(plugin_config("viewer", &server.uri())).await;

        assert_eq!(plugin.status, PluginStatus::Ok);
        let manifest = plugin.manifest.unwrap();
        assert_eq!(manifest.version, "0.1.0");
        assert_eq!(manifest.ui.unwrap().title, "Now playing");
    }

    #[tokio::test]
    async fn honors_base_path_in_plugin_url() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/viewer/manifest.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "name": "viewer", "version": "0.1.0", "api_version": 1
            })))
            .mount(&server)
            .await;

        let plugin = load_one(plugin_config(
            "viewer",
            &format!("{}/viewer/", server.uri()),
        ))
        .await;

        assert_eq!(plugin.status, PluginStatus::Ok);
    }

    #[tokio::test]
    async fn rejects_unsupported_api_version() {
        let server = MockServer::start().await;
        serve_manifest(
            &server,
            serde_json::json!({ "name": "viewer", "version": "2.0.0", "api_version": 2 }),
        )
        .await;

        let plugin = load_one(plugin_config("viewer", &server.uri())).await;

        assert_eq!(plugin.status, PluginStatus::Incompatible { api_version: 2 });
    }

    #[tokio::test]
    async fn rejects_manifest_with_other_name() {
        let server = MockServer::start().await;
        serve_manifest(
            &server,
            serde_json::json!({ "name": "other", "version": "0.1.0", "api_version": 1 }),
        )
        .await;

        let plugin = load_one(plugin_config("viewer", &server.uri())).await;

        assert!(matches!(plugin.status, PluginStatus::Invalid(_)));
    }

    #[tokio::test]
    async fn reports_malformed_manifest_as_invalid() {
        let server = MockServer::start().await;
        serve_manifest(&server, serde_json::json!({ "name": "viewer" })).await;

        let plugin = load_one(plugin_config("viewer", &server.uri())).await;

        assert!(matches!(plugin.status, PluginStatus::Invalid(_)));
        assert!(plugin.manifest.is_none());
    }

    #[tokio::test]
    async fn reports_http_errors_as_unreachable() {
        let server = MockServer::start().await;
        // No mock mounted: wiremock answers 404.

        let plugin = load_one(plugin_config("viewer", &server.uri())).await;

        assert!(matches!(plugin.status, PluginStatus::Unreachable(_)));
    }

    #[tokio::test]
    async fn skips_disabled_and_invalid_plugins_without_requests() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(500))
            .expect(0)
            .mount(&server)
            .await;
        let mut disabled = plugin_config("viewer", &server.uri());
        disabled.enabled = false;
        let invalid = plugin_config("Bad Name", &server.uri());

        let manager = PluginManager::new(reqwest::Client::new());
        manager.load(&[disabled, invalid]).await;
        let plugins = manager.plugins().await;

        assert_eq!(plugins[0].status, PluginStatus::Disabled);
        assert!(matches!(plugins[1].status, PluginStatus::Invalid(_)));
    }

    #[tokio::test]
    async fn marks_duplicate_names_invalid() {
        let server = MockServer::start().await;
        serve_manifest(
            &server,
            serde_json::json!({ "name": "viewer", "version": "0.1.0", "api_version": 1 }),
        )
        .await;
        let config = plugin_config("viewer", &server.uri());

        let manager = PluginManager::new(reqwest::Client::new());
        manager.load(&[config.clone(), config]).await;
        let plugins = manager.plugins().await;

        assert_eq!(plugins[0].status, PluginStatus::Ok);
        assert!(matches!(plugins[1].status, PluginStatus::Invalid(_)));
    }

    #[tokio::test]
    async fn authenticates_only_usable_plugins() {
        let server = MockServer::start().await;
        serve_manifest(
            &server,
            serde_json::json!({ "name": "viewer", "version": "0.1.0", "api_version": 1 }),
        )
        .await;
        let viewer = plugin_config("viewer", &server.uri());
        let mut offline = plugin_config("offline", "http://127.0.0.1:9");
        offline.token = Password::from("offline-token");
        let mut disabled = plugin_config("off", &server.uri());
        disabled.token = Password::from("disabled-token");
        disabled.enabled = false;

        let manager = PluginManager::new(reqwest::Client::new());
        manager.load(&[viewer, offline, disabled]).await;

        assert_eq!(
            manager.authenticate("secret").await.as_deref(),
            Some("viewer")
        );
        assert_eq!(
            manager.authenticate("offline-token").await.as_deref(),
            Some("offline")
        );
        assert_eq!(manager.authenticate("disabled-token").await, None);
        assert_eq!(manager.authenticate("secre").await, None);
        assert_eq!(manager.authenticate("").await, None);
    }

    #[test]
    fn compares_secrets() {
        assert!(constant_time_eq("secret", "secret"));
        assert!(!constant_time_eq("secret", "secreT"));
        assert!(!constant_time_eq("secret", "secrets"));
    }

    #[tokio::test]
    async fn reload_replaces_previous_plugins() {
        let manager = PluginManager::new(reqwest::Client::new());
        let mut disabled = plugin_config("viewer", "http://127.0.0.1:9");
        disabled.enabled = false;
        manager.load(&[disabled]).await;

        manager.load(&[]).await;

        assert!(manager.plugins().await.is_empty());
    }
}
