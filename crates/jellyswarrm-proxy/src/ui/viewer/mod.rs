//! fork(viewer): show who is watching what right now, in the admin UI.
//!
//! Fork-only feature. To keep merges with upstream small, it lives in its own
//! files and reads sessions only through `ClientSessionService::all()`. The
//! upstream files it touches are marked with `fork(viewer)`:
//! `ui/mod.rs` (routes), `templates/admin/index.html` (tab) and
//! `client_sessions/service.rs` (`all()`).
use askama::Template;
use axum::{
    extract::State,
    http::StatusCode,
    response::{Html, IntoResponse, Response},
    routing::get,
    Router,
};
use chrono::{DateTime, Duration, Utc};
use serde_json::Value;
use tracing::error;

use crate::{
    client_sessions::ClientSession,
    http::state::{AppState, ManagementState},
};

#[cfg(test)]
mod tests;

/// Clients report progress about every 10 seconds, even while paused, but a
/// closed browser tab sends no stop report. A playback without a report for
/// this long is treated as ended.
const STALE_AFTER_SECONDS: i64 = 60;
const TICKS_PER_SECOND: i64 = 10_000_000;

/// Admin-only routes, merged into the admin router before `require_admin`.
pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/viewer", get(page))
        .route("/viewer/list", get(list))
}

#[derive(Template)]
#[template(path = "viewer/page.html")]
struct PageTemplate {
    ui_route: String,
}

#[derive(Template)]
#[template(path = "viewer/list.html")]
struct ListTemplate {
    playbacks: Vec<Playback>,
    idle: usize,
}

/// One row of the viewer: display strings only, never tokens or IDs.
#[derive(Debug, PartialEq)]
pub(crate) struct Playback {
    pub(crate) user: String,
    pub(crate) title: String,
    pub(crate) subtitle: String,
    pub(crate) device: String,
    pub(crate) client: String,
    pub(crate) position: String,
    pub(crate) percent: u8,
    pub(crate) paused: bool,
}

async fn page(State(state): State<ManagementState>) -> Response {
    render(PageTemplate {
        ui_route: state.paths.ui_route(),
    })
}

async fn list(State(state): State<ManagementState>) -> Response {
    let sessions = state.sessions.sessions.all().await;
    let (playbacks, idle) = now_playing(&sessions, Utc::now());
    render(ListTemplate { playbacks, idle })
}

fn render(template: impl Template) -> Response {
    match template.render() {
        Ok(html) => Html(html).into_response(),
        Err(e) => {
            error!("Failed to render viewer template: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, "Template error").into_response()
        }
    }
}

/// Current playbacks, sorted by user and device so rows don't jump between
/// refreshes, plus the number of other active clients.
pub(crate) fn now_playing(
    sessions: &[ClientSession],
    now: DateTime<Utc>,
) -> (Vec<Playback>, usize) {
    let stale_before = now - Duration::seconds(STALE_AFTER_SECONDS);
    let (playing, idle): (Vec<_>, Vec<_>) = sessions.iter().partition(|s| {
        !s.now_playing.is_null() && s.last_playback.is_some_and(|at| at >= stale_before)
    });
    let mut playbacks: Vec<_> = playing.into_iter().map(playback).collect();
    playbacks.sort_by(|a, b| (&a.user, &a.device).cmp(&(&b.user, &b.device)));
    (playbacks, idle.len())
}

fn playback(session: &ClientSession) -> Playback {
    let item = &session.now_playing;
    let (title, subtitle) = describe(item);
    let position = ticks(&session.play_state["PositionTicks"]);
    let runtime = ticks(&item["RunTimeTicks"]);
    let percent = match (position, runtime) {
        (Some(position), Some(runtime)) if runtime > 0 => {
            (position.saturating_mul(100) / runtime).clamp(0, 100) as u8
        }
        _ => 0,
    };
    let position = match (position, runtime) {
        (Some(position), Some(runtime)) => format!("{} / {}", clock(position), clock(runtime)),
        (Some(position), None) => clock(position),
        _ => String::new(),
    };
    Playback {
        user: session.user_name.clone(),
        title,
        subtitle,
        device: readable(&session.device.device),
        client: format!(
            "{} {}",
            readable(&session.device.client),
            session.device.version
        )
        .trim()
        .to_string(),
        position,
        percent,
        paused: session.play_state["IsPaused"].as_bool().unwrap_or(false),
    }
}

/// Title and subtitle from the cached item metadata. Without metadata, the
/// session only knows the item ID, which means nothing to a person.
fn describe(item: &Value) -> (String, String) {
    let text = |key: &str| item[key].as_str().unwrap_or_default().to_string();
    let name = text("Name");
    if name.is_empty() {
        return ("Unknown item".into(), String::new());
    }
    match item["Type"].as_str() {
        Some("Episode") => {
            let episode = match (
                item["ParentIndexNumber"].as_i64(),
                item["IndexNumber"].as_i64(),
            ) {
                (Some(season), Some(episode)) => format!("S{season:02}E{episode:02} · {name}"),
                _ => name.clone(),
            };
            match text("SeriesName") {
                series if series.is_empty() => (name, String::new()),
                series => (series, episode),
            }
        }
        Some("Audio") => {
            let artists: Vec<_> = item["Artists"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect();
            let by = [artists.join(", "), text("Album")]
                .into_iter()
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join(" · ");
            (name, by)
        }
        _ => (name, String::new()),
    }
}

fn ticks(value: &Value) -> Option<i64> {
    value.as_i64().filter(|ticks| *ticks >= 0)
}

/// `1:02:03` or `2:03`.
fn clock(ticks: i64) -> String {
    let seconds = ticks / TICKS_PER_SECOND;
    let (h, m, s) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// Clients send device and client names form-encoded, e.g. `Jellyfin+for+Android+TV`.
fn readable(value: &str) -> String {
    value.replace('+', " ")
}
