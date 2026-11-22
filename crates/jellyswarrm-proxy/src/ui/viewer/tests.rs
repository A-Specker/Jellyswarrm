//! fork(viewer): tests for the admin viewer.
use super::*;
use crate::{
    client_sessions::{connections::ConnectionHub, ClientSessionService},
    http_tests::support::Fixture,
    protocol::device::Device,
};
use reqwest::StatusCode as HttpStatus;
use serde_json::json;

const HOUR: i64 = 3600 * TICKS_PER_SECOND;

fn device(client: &str, name: &str, id: &str) -> Device {
    Device {
        client: client.into(),
        device: name.into(),
        device_id: id.into(),
        version: "0.20.0".into(),
    }
}

fn episode() -> Value {
    json!({
        "Id": "episode-1", "Name": "Pilot", "Type": "Episode", "SeriesName": "Show",
        "ParentIndexNumber": 1, "IndexNumber": 2, "RunTimeTicks": HOUR
    })
}

/// Alice watches an episode on her TV (paused at half time); Bob has a web
/// client open without playback.
async fn sessions(f: &Fixture, service: &ClientSessionService) -> String {
    let tv = device("Jellyfin+for+Android+TV", "Living+room", "tv");
    let id = service.ensure(&f.user, "alice-token", Some(&tv)).await;
    service
        .cache_media_response("alice-token", &episode())
        .await;
    service
        .report(
            &id,
            false,
            &json!({"ItemId": "episode-1", "PositionTicks": HOUR / 2, "IsPaused": true}),
        )
        .await;
    let web = device("Jellyfin Web", "Firefox", "web");
    service.ensure(&f.other, "bob-token", Some(&web)).await;
    id
}

#[tokio::test]
async fn lists_playbacks_with_readable_names_and_counts_idle_clients() {
    let f = Fixture::new().await;
    let service = ClientSessionService::new(ConnectionHub::default());
    sessions(&f, &service).await;

    let (playbacks, idle) = now_playing(&service.all().await, Utc::now());

    assert_eq!(
        playbacks,
        vec![Playback {
            user: "alice".into(),
            title: "Show".into(),
            subtitle: "S01E02 · Pilot".into(),
            device: "Living room".into(),
            client: "Jellyfin for Android TV 0.20.0".into(),
            position: "30:00 / 1:00:00".into(),
            percent: 50,
            paused: true,
        }]
    );
    assert_eq!(idle, 1);
}

#[tokio::test]
async fn a_playback_without_recent_report_or_after_stop_is_not_listed() {
    let f = Fixture::new().await;
    let service = ClientSessionService::new(ConnectionHub::default());
    let id = sessions(&f, &service).await;

    // A closed browser tab sends no stop: the playback just goes silent.
    let later = Utc::now() + Duration::seconds(STALE_AFTER_SECONDS + 1);
    let (playbacks, idle) = now_playing(&service.all().await, later);
    assert!(playbacks.is_empty());
    assert_eq!(idle, 2);

    service
        .report(&id, true, &json!({"ItemId": "episode-1"}))
        .await;
    let (playbacks, _) = now_playing(&service.all().await, Utc::now());
    assert!(playbacks.is_empty());
}

#[test]
fn describes_items_by_type() {
    assert_eq!(
        describe(&json!({"Id": "x"})),
        ("Unknown item".into(), String::new())
    );
    assert_eq!(
        describe(&json!({"Name": "Movie", "Type": "Movie"})),
        ("Movie".into(), String::new())
    );
    assert_eq!(
        describe(&json!({"Name": "Pilot", "Type": "Episode", "SeriesName": "Show"})),
        ("Show".into(), "Pilot".into())
    );
    assert_eq!(
        describe(&json!({"Name": "Song", "Type": "Audio", "Artists": ["A", "B"], "Album": "LP"})),
        ("Song".into(), "A, B · LP".into())
    );
}

#[test]
fn formats_positions_as_clock_times() {
    assert_eq!(clock(0), "0:00");
    assert_eq!(clock(65 * TICKS_PER_SECOND), "1:05");
    assert_eq!(
        clock(HOUR + 2 * 60 * TICKS_PER_SECOND + 3 * TICKS_PER_SECOND),
        "1:02:03"
    );
}

#[test]
fn escapes_client_supplied_names() {
    let html = ListTemplate {
        playbacks: vec![Playback {
            user: "alice".into(),
            title: "<script>alert(1)</script>".into(),
            subtitle: String::new(),
            device: "<img src=x>".into(),
            client: "Web".into(),
            position: String::new(),
            percent: 0,
            paused: false,
        }],
        idle: 0,
    }
    .render()
    .unwrap();
    // The text is still shown, only as escaped entities.
    assert!(html.contains("alert(1)"));
    assert!(!html.contains("<script>"));
    assert!(!html.contains("<img"));
}

/// Log into the management UI and return the session cookie.
async fn ui_login(f: &Fixture, username: &str, password: &str) -> String {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let response = client
        .post(format!("{}/ui/login", f.url))
        .form(&[("username", username), ("password", password)])
        .send()
        .await
        .unwrap();
    let location = response.headers()["location"].to_str().unwrap().to_string();
    assert!(!location.contains("/login"), "login failed: {location}");
    response
        .headers()
        .get_all("set-cookie")
        .iter()
        .map(|c| c.to_str().unwrap().split(';').next().unwrap().to_string())
        .collect::<Vec<_>>()
        .join("; ")
}

async fn get(f: &Fixture, path: &str, cookie: Option<&str>) -> reqwest::Response {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let mut request = client.get(format!("{}/ui{path}", f.url));
    if let Some(cookie) = cookie {
        request = request.header("cookie", cookie);
    }
    request.send().await.unwrap()
}

#[tokio::test]
async fn only_admins_can_open_the_viewer() {
    let f = Fixture::new().await;
    sessions(&f, &f.state.sessions.sessions).await;

    let admin = ui_login(&f, "admin", "jellyswarrm").await;
    let response = get(&f, "/viewer/list", Some(&admin)).await;
    assert_eq!(response.status(), HttpStatus::OK);
    let html = response.text().await.unwrap();
    assert!(html.contains("S01E02 · Pilot"), "{html}");
    assert!(!html.contains("alice-token"));

    let page = get(&f, "/viewer", Some(&admin)).await;
    assert_eq!(page.status(), HttpStatus::OK);
    assert!(page.text().await.unwrap().contains("/viewer/list"));

    let index = get(&f, "", Some(&admin)).await.text().await.unwrap();
    assert!(index.contains("/viewer\""), "admin index has no viewer tab");

    let user = ui_login(&f, "alice", "password").await;
    let response = get(&f, "/viewer/list", Some(&user)).await;
    assert_eq!(response.status(), HttpStatus::FORBIDDEN);

    let response = get(&f, "/viewer/list", None).await;
    assert_ne!(response.status(), HttpStatus::OK);
    assert!(!response.text().await.unwrap().contains("Pilot"));
}

#[test]
fn api_needs_a_configured_key_sent_as_bearer() {
    let header = |value: &str| HeaderValue::from_str(value).unwrap();
    let key = Some("0123456789abcdef");
    assert_eq!(
        authorize(None, Some(&header("Bearer x"))),
        Err(StatusCode::NOT_FOUND)
    );
    assert_eq!(authorize(Some("  "), None), Err(StatusCode::NOT_FOUND));
    assert_eq!(authorize(key, None), Err(StatusCode::UNAUTHORIZED));
    for wrong in [
        "0123456789abcdef",
        "Bearer 0123456789abcdeX",
        "Bearer 0123456789abcde",
        "Basic 0123456789abcdef",
    ] {
        assert_eq!(
            authorize(key, Some(&header(wrong))),
            Err(StatusCode::UNAUTHORIZED),
            "{wrong}"
        );
    }
    assert_eq!(
        authorize(key, Some(&header("Bearer 0123456789abcdef"))),
        Ok(())
    );
}

/// The only test that touches `JELLYSWARRM_VIEWER_API_KEY`, so it can set and
/// remove it without racing other tests.
#[tokio::test]
async fn api_serves_json_with_the_key_and_opens_nothing_else() {
    let f = Fixture::new().await;
    sessions(&f, &f.state.sessions.sessions).await;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let request = |path: &str, key: Option<&str>| {
        let mut request = client.get(format!("{}/ui{path}", f.url));
        if let Some(key) = key {
            request = request.bearer_auth(key);
        }
        request.send()
    };
    let key = "test-viewer-api-key-0123456789";

    std::env::remove_var(API_KEY_ENV);
    let response = request("/viewer/now-playing", Some(key)).await.unwrap();
    assert_eq!(response.status(), HttpStatus::NOT_FOUND);

    std::env::set_var(API_KEY_ENV, key);
    let response = request("/viewer/now-playing", None).await.unwrap();
    assert_eq!(response.status(), HttpStatus::UNAUTHORIZED);
    // An admin login is not a key.
    let admin = ui_login(&f, "admin", "jellyswarrm").await;
    let response = get(&f, "/viewer/now-playing", Some(&admin)).await;
    assert_eq!(response.status(), HttpStatus::UNAUTHORIZED);

    let response = request("/viewer/now-playing", Some(key)).await.unwrap();
    assert_eq!(response.status(), HttpStatus::OK);
    let body: Value = response.json().await.unwrap();
    assert_eq!(
        body,
        json!({
            "count": 1,
            "idle": 1,
            "playbacks": [{
                "user": "alice",
                "title": "Show",
                "subtitle": "S01E02 · Pilot",
                "device": "Living room",
                "client": "Jellyfin for Android TV 0.20.0",
                "position": "30:00 / 1:00:00",
                "percent": 50,
                "paused": true
            }]
        })
    );

    // The key does not open the admin UI.
    for path in ["/viewer/list", "/viewer", "/users/list"] {
        let response = request(path, Some(key)).await.unwrap();
        assert_ne!(response.status(), HttpStatus::OK, "{path}");
    }
    std::env::remove_var(API_KEY_ENV);
}
