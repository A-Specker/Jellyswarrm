//! Plugin events: published from the proxy, streamed to plugins as Server-Sent
//! Events. Delivery is best effort, see `docs/plugins/decisions/0005-events-via-sse.md`.
use std::convert::Infallible;

use axum::{
    extract::State,
    response::sse::{Event, KeepAlive, Sse},
};
use chrono::{DateTime, Utc};
use futures_util::{stream, Stream};
use serde::Serialize;
use serde_json::Value;
use tokio::sync::broadcast::error::RecvError;
use tracing::warn;

use super::dto::{item_server, ServerRef, UserRef};
use crate::{processors::request_analyzer::PlaybackSessionAction, AppState};

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct PlaybackEvent {
    pub session_id: String,
    pub user: UserRef,
    pub item_id: String,
    pub server: Option<ServerRef>,
    pub position_ticks: Option<i64>,
    pub is_paused: bool,
    pub at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackPhase {
    Started,
    Progress,
    Stopped,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PluginEvent {
    Playback(PlaybackPhase, PlaybackEvent),
}

impl PluginEvent {
    fn to_sse(&self) -> Event {
        let (name, payload) = match self {
            Self::Playback(PlaybackPhase::Started, event) => ("playback.started", event),
            Self::Playback(PlaybackPhase::Progress, event) => ("playback.progress", event),
            Self::Playback(PlaybackPhase::Stopped, event) => ("playback.stopped", event),
        };
        Event::default()
            .event(name)
            .json_data(payload)
            .expect("plugin events serialize to JSON")
    }
}

/// Publish a playback report that already passed the proxy pipeline. Cheap when
/// no plugin is subscribed; never fails the request it was called from.
pub async fn publish_playback(
    state: &AppState,
    session_id: &str,
    action: PlaybackSessionAction,
    report: &Value,
) {
    if !state.plugins.has_subscribers() {
        return;
    }
    let Some(session) = state.client_sessions.get(session_id).await else {
        return;
    };
    let field = |name: &str| {
        report.as_object().and_then(|object| {
            object
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case(name))
                .map(|(_, value)| value)
        })
    };
    let Some(item_id) = field("ItemId").and_then(Value::as_str) else {
        return;
    };

    let event = PlaybackEvent {
        session_id: session.id,
        user: UserRef {
            id: session.user_id,
            name: session.user_name,
        },
        item_id: item_id.to_string(),
        server: item_server(state, item_id).await,
        position_ticks: field("PositionTicks").and_then(Value::as_i64),
        is_paused: field("IsPaused").and_then(Value::as_bool).unwrap_or(false),
        at: Utc::now(),
    };
    let phase = match action {
        PlaybackSessionAction::Start => PlaybackPhase::Started,
        PlaybackSessionAction::Refresh => PlaybackPhase::Progress,
        PlaybackSessionAction::Remove => PlaybackPhase::Stopped,
    };
    state.plugins.publish(PluginEvent::Playback(phase, event));
}

/// `GET /plugin-api/v1/events`
pub async fn stream(
    State(state): State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let receiver = state.plugins.subscribe();
    let events = stream::unfold(receiver, |mut receiver| async move {
        let event = match receiver.recv().await {
            Ok(event) => event.to_sse(),
            // Tell the plugin it missed events so it can reload `/sessions`.
            Err(RecvError::Lagged(missed)) => {
                warn!("A plugin event subscriber fell behind and missed {missed} events");
                Event::default()
                    .event("stream.lagged")
                    .json_data(serde_json::json!({ "missed": missed }))
                    .expect("lag notice serializes to JSON")
            }
            Err(RecvError::Closed) => return None,
        };
        Some((Ok(event), receiver))
    });
    Sse::new(events).keep_alive(KeepAlive::default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::test_support::{movie, state, TOKEN};
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use serde_json::json;
    use std::time::Duration;
    use tower::ServiceExt;

    async fn playing_session(state: &AppState) -> String {
        let alice = state
            .user_authorization
            .create_user("alice", &"password".into())
            .await
            .unwrap();
        state
            .client_sessions
            .ensure(&alice, "alice-token", None)
            .await
    }

    #[tokio::test]
    async fn maps_playback_actions_to_events() {
        let state = state().await;
        let (server_id, item) = movie(&state).await;
        let session = playing_session(&state).await;
        let mut events = state.plugins.subscribe();
        // Field names in Jellyfin reports are matched case-insensitively.
        let report = json!({ "itemId": item, "PositionTicks": 42, "IsPaused": true });

        for action in [
            PlaybackSessionAction::Start,
            PlaybackSessionAction::Refresh,
            PlaybackSessionAction::Remove,
        ] {
            publish_playback(&state, &session, action, &report).await;
        }

        let mut received = Vec::new();
        for _ in 0..3 {
            let PluginEvent::Playback(phase, event) = events.recv().await.unwrap();
            received.push((phase, event));
        }
        let phases: Vec<_> = received.iter().map(|(phase, _)| *phase).collect();
        assert_eq!(
            phases,
            [
                PlaybackPhase::Started,
                PlaybackPhase::Progress,
                PlaybackPhase::Stopped
            ]
        );
        let event = &received[0].1;
        assert_eq!(event.session_id, session);
        assert_eq!(event.user.name, "alice");
        assert_eq!(event.item_id, item);
        assert_eq!(
            event.server,
            Some(ServerRef {
                id: server_id.to_string(),
                name: "Movies 1".to_string()
            })
        );
        assert_eq!(event.position_ticks, Some(42));
        assert!(event.is_paused);
    }

    #[tokio::test]
    async fn ignores_reports_without_item_or_session() {
        let state = state().await;
        let session = playing_session(&state).await;
        let mut events = state.plugins.subscribe();

        publish_playback(
            &state,
            &session,
            PlaybackSessionAction::Start,
            &json!({ "PositionTicks": 1 }),
        )
        .await;
        publish_playback(
            &state,
            "unknown-session",
            PlaybackSessionAction::Start,
            &json!({ "ItemId": "x" }),
        )
        .await;

        assert!(events.try_recv().is_err());
    }

    /// Read the SSE body until `needle` shows up.
    async fn read_until(body: &mut Body, needle: &str) -> String {
        let mut text = String::new();
        while !text.contains(needle) {
            let frame = tokio::time::timeout(Duration::from_secs(5), body.frame())
                .await
                .expect("event arrives in time")
                .expect("stream stays open")
                .unwrap();
            if let Ok(data) = frame.into_data() {
                text.push_str(&String::from_utf8_lossy(&data));
            }
        }
        text
    }

    async fn open_stream(state: &AppState) -> Body {
        let response = crate::plugins::router(state.clone())
            .oneshot(
                Request::builder()
                    .uri("/plugin-api/v1/events")
                    .header("authorization", format!("Bearer {TOKEN}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(response.headers()["content-type"], "text/event-stream");
        response.into_body()
    }

    #[tokio::test]
    async fn streams_events_as_sse() {
        let state = state().await;
        let (_, item) = movie(&state).await;
        let session = playing_session(&state).await;
        let mut body = open_stream(&state).await;

        publish_playback(
            &state,
            &session,
            PlaybackSessionAction::Start,
            &json!({ "ItemId": item }),
        )
        .await;

        let text = read_until(&mut body, "\n\n").await;
        assert!(text.contains("event: playback.started\n"), "{text}");
        let data = text
            .lines()
            .find_map(|line| line.strip_prefix("data: "))
            .unwrap();
        let data: Value = serde_json::from_str(data).unwrap();
        assert_eq!(data["item_id"], item);
        assert_eq!(data["user"]["name"], "alice");
    }

    #[tokio::test]
    async fn tells_slow_subscribers_about_missed_events() {
        let state = state().await;
        let (_, item) = movie(&state).await;
        let session = playing_session(&state).await;
        let mut body = open_stream(&state).await;

        // Overflow the per-subscriber buffer before the stream is read.
        for _ in 0..300 {
            publish_playback(
                &state,
                &session,
                PlaybackSessionAction::Refresh,
                &json!({ "ItemId": item }),
            )
            .await;
        }

        let text = read_until(&mut body, "event: stream.lagged").await;
        assert!(text.contains("\"missed\":"), "{text}");
    }
}
