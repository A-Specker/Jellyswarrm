//! Read-only plugin API v1. Maps internal state to stable DTOs; see
//! `docs/plugins/plugin-api-v1.md`. Never serialize internal types directly.
use axum::{
    extract::{Request, State},
    http::{header::AUTHORIZATION, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;
use tracing::{error, trace};

use super::dto::{item_server, ServerRef, UserRef};
use crate::AppState;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/plugin-api/v1/sessions", get(sessions))
        .route("/plugin-api/v1/users", get(users))
        .route("/plugin-api/v1/servers", get(servers))
        .route("/plugin-api/v1/events", get(super::events::stream))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            require_plugin_token,
        ))
        .with_state(state)
}

async fn require_plugin_token(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let token = req
        .headers()
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    let Some(token) = token else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    match state.plugins.authenticate(token).await {
        Some(plugin) => {
            trace!("Plugin '{plugin}' requested {}", req.uri().path());
            next.run(req).await
        }
        None => StatusCode::UNAUTHORIZED.into_response(),
    }
}

#[derive(Debug, Serialize, PartialEq)]
struct DeviceDto {
    client: String,
    name: String,
    id: String,
    version: String,
}

#[derive(Debug, Serialize, PartialEq)]
struct NowPlayingDto {
    item_id: String,
    name: Option<String>,
    #[serde(rename = "type")]
    item_type: Option<String>,
    series_name: Option<String>,
    /// `None` when the item has no single owning server (e.g. a merged item).
    server: Option<ServerRef>,
    position_ticks: Option<i64>,
    is_paused: bool,
}

#[derive(Debug, Serialize, PartialEq)]
struct SessionDto {
    id: String,
    user: UserRef,
    device: DeviceDto,
    last_activity: DateTime<Utc>,
    now_playing: Option<NowPlayingDto>,
}

#[derive(Debug, Serialize, PartialEq)]
struct ServerDto {
    id: String,
    name: String,
    healthy: bool,
}

async fn sessions(State(state): State<AppState>) -> Json<Vec<SessionDto>> {
    let mut result = Vec::new();
    for session in state.client_sessions.all().await {
        let now_playing = now_playing(&state, &session.now_playing, &session.play_state).await;
        result.push(SessionDto {
            id: session.id,
            user: UserRef {
                id: session.user_id,
                name: session.user_name,
            },
            device: DeviceDto {
                client: session.device.client,
                name: session.device.device,
                id: session.device.device_id,
                version: session.device.version,
            },
            last_activity: session.last_activity,
            now_playing,
        });
    }
    Json(result)
}

async fn now_playing(state: &AppState, item: &Value, play_state: &Value) -> Option<NowPlayingDto> {
    let item_id = item["Id"].as_str()?;
    let text = |value: &Value| value.as_str().map(str::to_string);
    let server = item_server(state, item_id).await;

    Some(NowPlayingDto {
        item_id: item_id.to_string(),
        name: text(&item["Name"]),
        item_type: text(&item["Type"]),
        series_name: text(&item["SeriesName"]),
        server,
        position_ticks: play_state["PositionTicks"].as_i64(),
        is_paused: play_state["IsPaused"].as_bool().unwrap_or(false),
    })
}

async fn users(State(state): State<AppState>) -> Result<Json<Vec<UserRef>>, StatusCode> {
    let users = state.user_authorization.list_users().await.map_err(|e| {
        error!("Failed to list users for plugin API: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    Ok(Json(
        users
            .into_iter()
            .map(|user| UserRef {
                id: user.id,
                name: user.original_username,
            })
            .collect(),
    ))
}

async fn servers(State(state): State<AppState>) -> Result<Json<Vec<ServerDto>>, StatusCode> {
    let servers = state.server_storage.list_servers().await.map_err(|e| {
        error!("Failed to list servers for plugin API: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    let mut result = Vec::with_capacity(servers.len());
    for server in servers {
        let healthy = state
            .server_storage
            .server_status(server.id)
            .await
            .is_healthy();
        result.push(ServerDto {
            id: server.id.to_string(),
            name: server.name,
            healthy,
        });
    }
    Ok(Json(result))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::MediaStreamingMode,
        plugins::test_support::{movie, state, TOKEN},
        user_authorization_service::Device,
    };
    use axum::body::Body;
    use http_body_util::BodyExt;
    use serde_json::json;
    use tower::ServiceExt;

    async fn get(state: &AppState, path: &str, token: Option<&str>) -> (StatusCode, Value) {
        let mut request = Request::builder().uri(path);
        if let Some(token) = token {
            request = request.header(AUTHORIZATION, format!("Bearer {token}"));
        }
        let response = router(state.clone())
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json = serde_json::from_slice(&body).unwrap_or(Value::Null);
        (status, json)
    }

    #[tokio::test]
    async fn rejects_missing_or_wrong_token() {
        let state = state().await;

        for token in [None, Some("wrong"), Some("")] {
            for path in [
                "/plugin-api/v1/sessions",
                "/plugin-api/v1/users",
                "/plugin-api/v1/servers",
                "/plugin-api/v1/events",
            ] {
                let (status, _) = get(&state, path, token).await;
                assert_eq!(status, StatusCode::UNAUTHORIZED, "{path} with {token:?}");
            }
        }
    }

    #[tokio::test]
    async fn lists_sessions_of_all_users_with_now_playing() {
        let state = state().await;
        let (server_id, item) = movie(&state).await;
        let alice = state
            .user_authorization
            .create_user("alice", &"password".into())
            .await
            .unwrap();
        let bob = state
            .user_authorization
            .create_user("bob", &"password".into())
            .await
            .unwrap();
        let device = Device {
            client: "Jellyfin Android TV".into(),
            device: "Living room".into(),
            device_id: "tv-1".into(),
            version: "0.19.10".into(),
        };
        let playing = state
            .client_sessions
            .ensure(&alice, "alice-token", Some(&device))
            .await;
        state
            .client_sessions
            .cache_media_response(
                "alice-token",
                &json!({"Id": item, "Name": "Big Buck Bunny", "Type": "Movie"}),
            )
            .await;
        state
            .client_sessions
            .report(
                &playing,
                false,
                &json!({"ItemId": item, "PositionTicks": 1234, "IsPaused": true}),
            )
            .await;
        state.client_sessions.ensure(&bob, "bob-token", None).await;

        let (status, body) = get(&state, "/plugin-api/v1/sessions", Some(TOKEN)).await;

        assert_eq!(status, StatusCode::OK);
        let sessions = body.as_array().unwrap();
        assert_eq!(sessions.len(), 2);
        let alice_session = sessions
            .iter()
            .find(|s| s["user"]["name"] == "alice")
            .unwrap();
        assert_eq!(alice_session["device"]["name"], "Living room");
        assert_eq!(
            alice_session["now_playing"],
            json!({
                "item_id": item,
                "name": "Big Buck Bunny",
                "type": "Movie",
                "series_name": null,
                "server": { "id": server_id.to_string(), "name": "Movies 1" },
                "position_ticks": 1234,
                "is_paused": true
            })
        );
        let bob_session = sessions
            .iter()
            .find(|s| s["user"]["name"] == "bob")
            .unwrap();
        assert_eq!(bob_session["now_playing"], Value::Null);
        // Internal credentials never leave the process.
        let raw = body.to_string();
        assert!(!raw.contains("alice-token") && !raw.contains("bob-token"));
    }

    #[tokio::test]
    async fn lists_users_without_credentials() {
        let state = state().await;
        let alice = state
            .user_authorization
            .create_user("alice", &"password".into())
            .await
            .unwrap();

        let (status, body) = get(&state, "/plugin-api/v1/users", Some(TOKEN)).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, json!([{ "id": alice.id, "name": "alice" }]));
    }

    #[tokio::test]
    async fn lists_servers_without_urls() {
        let state = state().await;
        let id = state
            .server_storage
            .add_server(
                "Movies 1",
                "http://10.0.0.5:8096",
                100,
                MediaStreamingMode::Proxy,
            )
            .await
            .unwrap();

        let (status, body) = get(&state, "/plugin-api/v1/servers", Some(TOKEN)).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            body,
            json!([{ "id": id.to_string(), "name": "Movies 1", "healthy": false }])
        );
    }
}
