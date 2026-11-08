//! Read-only plugin queries. Every result is a plugin API DTO: a stable,
//! versioned shape that never exposes internal types, credentials, server URLs
//! or backend IDs.
use crate::{
    auth::UserAuthorizationService, client_sessions::ClientSessionService,
    config::PluginCredentials, domain::ids::ServerId, libraries::VirtualLibraryService,
    servers::ServerStorageService,
};
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;
use std::{collections::HashMap, sync::Arc};

pub(crate) mod dto {
    use super::*;

    #[derive(Debug, Serialize, PartialEq)]
    pub(crate) struct Server {
        pub(crate) id: ServerId,
        pub(crate) name: String,
        pub(crate) priority: i32,
        pub(crate) healthy: bool,
    }

    #[derive(Debug, Serialize, PartialEq)]
    pub(crate) struct User {
        pub(crate) id: String,
        pub(crate) name: String,
        pub(crate) created_at: DateTime<Utc>,
    }

    #[derive(Debug, Serialize, PartialEq)]
    pub(crate) struct UserRef {
        pub(crate) id: String,
        pub(crate) name: String,
    }

    #[derive(Debug, Serialize, PartialEq)]
    pub(crate) struct Device {
        pub(crate) id: String,
        pub(crate) name: String,
        pub(crate) client: String,
        pub(crate) version: String,
    }

    #[derive(Debug, Serialize, PartialEq)]
    pub(crate) struct NowPlaying {
        /// Virtual item ID, as clients see it.
        pub(crate) item_id: String,
        /// `None` until Jellyswarrm has served the item's metadata to the client.
        pub(crate) name: Option<String>,
        pub(crate) r#type: Option<String>,
        pub(crate) series_name: Option<String>,
        pub(crate) position_ticks: Option<i64>,
        pub(crate) is_paused: bool,
    }

    #[derive(Debug, Serialize, PartialEq)]
    pub(crate) struct Session {
        pub(crate) id: String,
        pub(crate) user: UserRef,
        pub(crate) device: Device,
        pub(crate) last_activity: DateTime<Utc>,
        pub(crate) now_playing: Option<NowPlaying>,
    }

    #[derive(Debug, Serialize, PartialEq)]
    pub(crate) struct LibraryMember {
        pub(crate) server: ServerRef,
        pub(crate) name: String,
    }

    #[derive(Debug, Serialize, PartialEq)]
    pub(crate) struct ServerRef {
        pub(crate) id: ServerId,
        /// `None` if the server was removed after the library was assigned.
        pub(crate) name: Option<String>,
    }

    #[derive(Debug, Serialize, PartialEq)]
    pub(crate) struct Library {
        /// Virtual library ID, as clients see it.
        pub(crate) id: String,
        pub(crate) name: String,
        pub(crate) collection_type: Option<String>,
        pub(crate) members: Vec<LibraryMember>,
    }
}

#[derive(Clone)]
pub(crate) struct PluginQueries {
    pub(crate) credentials: PluginCredentials,
    servers: Arc<ServerStorageService>,
    users: Arc<UserAuthorizationService>,
    sessions: Arc<ClientSessionService>,
    libraries: Arc<VirtualLibraryService>,
}

impl PluginQueries {
    pub(crate) fn new(
        credentials: PluginCredentials,
        servers: Arc<ServerStorageService>,
        users: Arc<UserAuthorizationService>,
        sessions: Arc<ClientSessionService>,
        libraries: Arc<VirtualLibraryService>,
    ) -> Self {
        Self {
            credentials,
            servers,
            users,
            sessions,
            libraries,
        }
    }

    /// Health reasons are omitted: transport errors can contain server URLs.
    pub(crate) async fn servers(&self) -> Result<Vec<dto::Server>, sqlx::Error> {
        let mut result = Vec::new();
        for server in self.servers.list_servers().await? {
            result.push(dto::Server {
                healthy: self.servers.server_status(server.id).await.is_healthy(),
                id: server.id,
                name: server.name,
                priority: server.priority,
            });
        }
        Ok(result)
    }

    pub(crate) async fn users(&self) -> Result<Vec<dto::User>, sqlx::Error> {
        Ok(self
            .users
            .list_users()
            .await?
            .into_iter()
            .map(|user| dto::User {
                id: user.id.to_string(),
                name: user.original_username,
                created_at: user.created_at,
            })
            .collect())
    }

    pub(crate) async fn sessions(&self) -> Vec<dto::Session> {
        self.sessions
            .all()
            .await
            .into_iter()
            .map(|session| dto::Session {
                now_playing: now_playing(&session.now_playing, &session.play_state),
                id: session.id,
                user: dto::UserRef {
                    id: session.user_id.to_string(),
                    name: session.user_name,
                },
                device: dto::Device {
                    id: session.device.device_id,
                    name: session.device.device,
                    client: session.device.client,
                    version: session.device.version,
                },
                last_activity: session.last_activity,
            })
            .collect()
    }

    pub(crate) async fn libraries(&self) -> Result<Vec<dto::Library>, sqlx::Error> {
        let server_names: HashMap<ServerId, String> = self
            .servers
            .list_servers()
            .await?
            .into_iter()
            .map(|server| (server.id, server.name))
            .collect();
        let mut result = Vec::new();
        for group in self.libraries.list_groups().await? {
            let members = self
                .libraries
                .list_members(&group.virtual_id)
                .await?
                .into_iter()
                .map(|member| dto::LibraryMember {
                    server: dto::ServerRef {
                        id: member.server_id,
                        name: server_names.get(&member.server_id).cloned(),
                    },
                    name: member.library_name,
                })
                .collect();
            result.push(dto::Library {
                id: group.virtual_id,
                name: group.name,
                collection_type: group.collection_type,
                members,
            });
        }
        Ok(result)
    }
}

/// Map the Jellyfin-shaped session state to the plugin contract.
fn now_playing(item: &Value, state: &Value) -> Option<dto::NowPlaying> {
    let text = |value: &Value, key: &str| value.get(key).and_then(Value::as_str).map(String::from);
    Some(dto::NowPlaying {
        item_id: text(item, "Id")?,
        name: text(item, "Name"),
        r#type: text(item, "Type"),
        series_name: text(item, "SeriesName"),
        position_ticks: state.get("PositionTicks").and_then(Value::as_i64),
        is_paused: state
            .get("IsPaused")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn idle_sessions_have_no_now_playing() {
        assert_eq!(now_playing(&Value::Null, &json!({"IsPaused": false})), None);
    }

    #[test]
    fn now_playing_keeps_only_contract_fields() {
        let item = json!({"Id": "abc", "Name": "Movie", "Type": "Movie", "Path": "/secret"});
        let state = json!({"PositionTicks": 42, "IsPaused": true});
        assert_eq!(
            now_playing(&item, &state),
            Some(dto::NowPlaying {
                item_id: "abc".into(),
                name: Some("Movie".into()),
                r#type: Some("Movie".into()),
                series_name: None,
                position_ticks: Some(42),
                is_paused: true,
            })
        );
    }

    #[test]
    fn metadata_is_optional_until_seen() {
        let playing = now_playing(&json!({"Id": "abc"}), &json!({})).unwrap();
        assert_eq!(playing.name, None);
        assert!(!playing.is_paused);
        assert_eq!(playing.position_ticks, None);
    }
}
