//! Plugin registrations: who may call the plugin API and what each plugin may read.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::HashSet, fmt};

/// Shortest accepted plugin token. Tokens are long-lived shared secrets.
pub(crate) const MIN_TOKEN_LENGTH: usize = 32;

/// What a plugin may read. Each scope unlocks one group of endpoints.
/// Variant names mirror the `<area>:<access>` strings used in the config.
#[allow(clippy::enum_variant_names)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) enum PluginScope {
    #[serde(rename = "servers:read")]
    ServersRead,
    #[serde(rename = "users:read")]
    UsersRead,
    #[serde(rename = "sessions:read")]
    SessionsRead,
    #[serde(rename = "libraries:read")]
    LibrariesRead,
}

impl PluginScope {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::ServersRead => "servers:read",
            Self::UsersRead => "users:read",
            Self::SessionsRead => "sessions:read",
            Self::LibrariesRead => "libraries:read",
        }
    }
}

/// Plugin secret. Never printed; compared in constant time.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct PluginToken(String);

impl PluginToken {
    /// Compare digests so the comparison time depends on neither token's content
    /// nor length.
    fn matches(&self, presented: &str) -> bool {
        let expected = Sha256::digest(self.0.as_bytes());
        let presented = Sha256::digest(presented.as_bytes());
        expected
            .iter()
            .zip(presented.iter())
            .fold(0u8, |diff, (a, b)| diff | (a ^ b))
            == 0
    }
}

impl From<&str> for PluginToken {
    fn from(token: &str) -> Self {
        Self(token.to_string())
    }
}

impl fmt::Debug for PluginToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PluginToken(***)")
    }
}

fn default_enabled() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PluginConfig {
    /// Unique, shown in logs: `[a-z0-9-]`.
    pub(crate) name: String,
    pub(crate) token: PluginToken,
    #[serde(default)]
    pub(crate) scopes: Vec<PluginScope>,
    #[serde(default = "default_enabled")]
    pub(crate) enabled: bool,
}

/// The authenticated caller of a plugin API request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PluginIdentity {
    pub(crate) name: String,
    scopes: HashSet<PluginScope>,
}

impl PluginIdentity {
    pub(crate) fn allows(&self, scope: PluginScope) -> bool {
        self.scopes.contains(&scope)
    }
}

/// Find the enabled plugin a token belongs to. Every registration is checked,
/// so the time taken doesn't reveal which plugin matched.
pub(crate) fn authenticate(plugins: &[PluginConfig], token: &str) -> Option<PluginIdentity> {
    let mut found = None;
    for plugin in plugins {
        if plugin.token.matches(token) && plugin.enabled && found.is_none() {
            found = Some(PluginIdentity {
                name: plugin.name.clone(),
                scopes: plugin.scopes.iter().copied().collect(),
            });
        }
    }
    found
}

/// Reject registrations that would be ambiguous or easy to guess.
pub(crate) fn validate(plugins: &[PluginConfig]) -> anyhow::Result<()> {
    let mut names = HashSet::new();
    let mut tokens = HashSet::new();
    for plugin in plugins {
        let name = &plugin.name;
        anyhow::ensure!(
            !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
            "plugin name {name:?} must only contain a-z, 0-9 and '-'"
        );
        anyhow::ensure!(names.insert(name), "plugin name {name:?} is used twice");
        anyhow::ensure!(
            plugin.token.0.len() >= MIN_TOKEN_LENGTH,
            "token of plugin {name:?} must have at least {MIN_TOKEN_LENGTH} characters"
        );
        anyhow::ensure!(
            tokens.insert(&plugin.token.0),
            "plugin {name:?} reuses another plugin's token"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "0123456789abcdef0123456789abcdef";

    fn plugin(name: &str, token: &str, scopes: &[PluginScope], enabled: bool) -> PluginConfig {
        PluginConfig {
            name: name.into(),
            token: token.into(),
            scopes: scopes.to_vec(),
            enabled,
        }
    }

    #[test]
    fn authenticates_enabled_plugins_with_their_scopes() {
        let plugins = [plugin("viewer", TOKEN, &[PluginScope::SessionsRead], true)];
        let identity = authenticate(&plugins, TOKEN).unwrap();
        assert_eq!(identity.name, "viewer");
        assert!(identity.allows(PluginScope::SessionsRead));
        assert!(!identity.allows(PluginScope::UsersRead));
    }

    #[test]
    fn rejects_unknown_tokens_and_disabled_plugins() {
        let plugins = [plugin("viewer", TOKEN, &[PluginScope::SessionsRead], false)];
        assert_eq!(authenticate(&plugins, TOKEN), None);
        assert_eq!(authenticate(&plugins, "wrong"), None);
        assert_eq!(authenticate(&[], TOKEN), None);
    }

    #[test]
    fn validation_rejects_bad_names_short_and_shared_tokens() {
        let other = "fedcba9876543210fedcba9876543210";
        assert!(validate(&[plugin("viewer-1", TOKEN, &[], true)]).is_ok());
        assert!(validate(&[plugin("Viewer", TOKEN, &[], true)]).is_err());
        assert!(validate(&[plugin("", TOKEN, &[], true)]).is_err());
        assert!(validate(&[plugin("viewer", "short", &[], true)]).is_err());
        assert!(validate(&[
            plugin("viewer", TOKEN, &[], true),
            plugin("viewer", other, &[], true)
        ])
        .is_err());
        assert!(validate(&[plugin("a", TOKEN, &[], true), plugin("b", TOKEN, &[], true)]).is_err());
    }

    #[test]
    fn config_uses_readable_scope_names_and_hides_tokens_in_debug() {
        let parsed: PluginConfig = toml::from_str(&format!(
            "name = \"viewer\"\ntoken = \"{TOKEN}\"\nscopes = [\"sessions:read\", \"servers:read\"]"
        ))
        .unwrap();
        assert!(parsed.enabled);
        assert_eq!(
            parsed.scopes,
            [PluginScope::SessionsRead, PluginScope::ServersRead]
        );
        assert!(!format!("{parsed:?}").contains(TOKEN));
    }
}
