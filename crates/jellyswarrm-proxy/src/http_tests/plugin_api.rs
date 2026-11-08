//! Plugin API v1 through the production router: tokens, scopes and DTO shapes.
use super::*;
use crate::config::{
    plugins::{PluginConfig, PluginScope},
    AppConfig,
};

const ALL: &str = "all-scopes-token-0123456789abcdef";
const SESSIONS_ONLY: &str = "sessions-only-token-0123456789abc";
const DISABLED: &str = "disabled-plugin-token-0123456789a";

fn plugin(name: &str, token: &str, scopes: &[PluginScope], enabled: bool) -> PluginConfig {
    PluginConfig {
        name: name.into(),
        token: token.into(),
        scopes: scopes.to_vec(),
        enabled,
    }
}

async fn fixture() -> Fixture {
    use PluginScope::*;
    Fixture::with_config(AppConfig {
        plugins: vec![
            plugin(
                "dashboard",
                ALL,
                &[ServersRead, UsersRead, SessionsRead, LibrariesRead],
                true,
            ),
            plugin("viewer", SESSIONS_ONLY, &[SessionsRead], true),
            plugin("old", DISABLED, &[ServersRead], false),
        ],
        ..Default::default()
    })
    .await
}

async fn get(f: &Fixture, path: &str, token: Option<&str>) -> reqwest::Response {
    let request = reqwest::Client::new().get(format!("{}/plugin-api/v1{path}", f.url));
    let request = match token {
        Some(token) => request.bearer_auth(token),
        None => request,
    };
    request.send().await.unwrap()
}

async fn json(f: &Fixture, path: &str, token: &str) -> Value {
    let response = get(f, path, Some(token)).await;
    assert_eq!(response.status(), StatusCode::OK, "{path}");
    assert_eq!(response.headers()["cache-control"], "no-store");
    response.json().await.unwrap()
}

#[tokio::test]
async fn rejects_missing_unknown_and_disabled_tokens() {
    let f = fixture().await;
    for token in [None, Some("wrong-token"), Some(DISABLED)] {
        let response = get(&f, "/servers", token).await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{token:?}");
        assert_eq!(response.headers()["www-authenticate"], "Bearer");
    }
    // A Jellyfin user token is not a plugin token.
    let response = f
        .request(
            reqwest::Method::GET,
            "/plugin-api/v1/servers",
            &f.user,
            "web",
        )
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    f.no_upstream_requests().await;
}

#[tokio::test]
async fn enforces_scopes_per_endpoint() {
    let f = fixture().await;
    for path in ["/servers", "/users", "/libraries"] {
        let response = get(&f, path, Some(SESSIONS_ONLY)).await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "{path}");
    }
    assert_eq!(
        get(&f, "/sessions", Some(SESSIONS_ONLY)).await.status(),
        StatusCode::OK
    );
}

#[tokio::test]
async fn servers_and_users_expose_no_urls_or_credentials() {
    let f = fixture().await;
    let servers = json(&f, "/servers", ALL).await;
    assert_eq!(servers.as_array().unwrap().len(), 2);
    assert_eq!(servers[0]["name"], "Server 0");
    assert_eq!(servers[0]["priority"], 100);
    assert!(servers[0]["healthy"].is_boolean());
    let text = servers.to_string();
    for upstream in &f.upstreams {
        assert!(!text.contains(&upstream.uri()));
    }

    let users = json(&f, "/users", ALL).await;
    let names: Vec<_> = users
        .as_array()
        .unwrap()
        .iter()
        .map(|user| user["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"alice") && names.contains(&"bob"));
    assert!(!users.to_string().contains(&f.user.virtual_key));
}

#[tokio::test]
async fn sessions_show_device_and_now_playing() {
    let f = fixture().await;
    assert_eq!(json(&f, "/sessions", SESSIONS_ONLY).await, json!([]));
    f.capabilities().await;

    let sessions = json(&f, "/sessions", SESSIONS_ONLY).await;
    assert_eq!(sessions.as_array().unwrap().len(), 1);
    let session = &sessions[0];
    assert_eq!(session["user"]["name"], "alice");
    assert_eq!(session["device"]["id"], "tv");
    assert_eq!(session["device"]["client"], "Test");
    assert_eq!(session["now_playing"], Value::Null);
    assert!(!sessions.to_string().contains(&f.user.virtual_key));

    let id = session["id"].as_str().unwrap();
    f.state
        .client_sessions
        .report(
            id,
            false,
            &json!({"ItemId": f.items[0], "PositionTicks": 12, "IsPaused": true}),
        )
        .await;
    let playing = &json(&f, "/sessions", SESSIONS_ONLY).await[0]["now_playing"];
    assert_eq!(playing["item_id"], f.items[0].as_str());
    assert_eq!(playing["position_ticks"], 12);
    assert_eq!(playing["is_paused"], true);
    assert_eq!(playing["name"], Value::Null);
}

#[tokio::test]
async fn libraries_list_virtual_libraries() {
    let f = fixture().await;
    assert_eq!(json(&f, "/libraries", ALL).await, json!([]));
    let group = f
        .state
        .virtual_library_service
        .create_group("Movies")
        .await
        .unwrap();
    let libraries = json(&f, "/libraries", ALL).await;
    assert_eq!(libraries[0]["id"], group.virtual_id.as_str());
    assert_eq!(libraries[0]["name"], "Movies");
    assert_eq!(libraries[0]["members"], json!([]));
}

#[tokio::test]
async fn plugin_api_is_mounted_below_the_url_prefix_only() {
    let f = fixture().await;
    let root = f.url.strip_suffix("/proxy").unwrap();
    let response = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap()
        .get(format!("{root}/plugin-api/v1/servers"))
        .bearer_auth(ALL)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT);
    assert_eq!(
        response.headers()["location"],
        "/proxy/plugin-api/v1/servers"
    );
}
