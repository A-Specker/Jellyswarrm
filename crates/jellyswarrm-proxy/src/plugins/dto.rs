//! JSON shapes shared by the plugin API and events. Changing a field here is a
//! breaking change of plugin API v1.
use serde::Serialize;
use tracing::error;

use crate::AppState;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct UserRef {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ServerRef {
    pub id: String,
    pub name: String,
}

/// The backend serving a virtual item, or `None` when no single server owns it
/// (e.g. a merged item with versions on several servers).
pub async fn item_server(state: &AppState, item_id: &str) -> Option<ServerRef> {
    match state
        .media_storage
        .get_media_mapping_with_server(item_id)
        .await
    {
        Ok(mapping) => mapping.map(|(_, server)| ServerRef {
            id: server.id.to_string(),
            name: server.name,
        }),
        Err(e) => {
            error!("Failed to resolve server for item {item_id}: {e}");
            None
        }
    }
}
