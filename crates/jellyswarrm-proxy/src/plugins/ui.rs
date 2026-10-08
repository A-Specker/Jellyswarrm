//! Plugin pages in the admin UI, see `docs/plugins/decisions/0006-ui-via-reverse-proxy.md`.
//! Mounted inside the admin routes, so every route here requires an admin login.
use askama::Template;
use axum::{
    body::Body,
    extract::{Path, Request, State},
    http::{header, HeaderMap, HeaderName, StatusCode},
    response::{Html, IntoResponse, Response},
    routing::{any, get},
    Router,
};
use tracing::{error, warn};

use super::manager::{PluginStatus, UiTab};
use crate::{url_helper::join_server_url, AppState};

/// Largest request body forwarded to a plugin page (forms, small uploads).
const MAX_BODY_BYTES: usize = 10 * 1024 * 1024;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/plugins", get(plugins_page))
        .route("/plugins/{name}", get(plugin_page))
        .route("/plugins/{name}/", any(proxy_root))
        .route("/plugins/{name}/{*path}", any(proxy))
}

struct PluginRow {
    name: String,
    url: String,
    version: Option<String>,
    status: &'static str,
    ok: bool,
    detail: Option<String>,
}

#[derive(Template)]
#[template(path = "plugins/list.html")]
struct PluginsPageTemplate {
    plugins: Vec<PluginRow>,
}

#[derive(Template)]
#[template(path = "plugins/page.html")]
struct PluginPageTemplate {
    title: String,
    src: String,
}

fn render(template: impl Template) -> Response {
    match template.render() {
        Ok(html) => Html(html).into_response(),
        Err(e) => {
            error!("Failed to render plugin template: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "Template error").into_response()
        }
    }
}

async fn plugins_page(State(state): State<AppState>) -> Response {
    let plugins = state
        .plugins
        .plugins()
        .await
        .into_iter()
        .map(|plugin| {
            let (status, detail) = match plugin.status {
                PluginStatus::Ok => ("Running", None),
                PluginStatus::Disabled => ("Disabled", None),
                PluginStatus::Unreachable(error) => ("Unreachable", Some(error)),
                PluginStatus::Incompatible { api_version } => (
                    "Incompatible",
                    Some(format!("needs plugin API v{api_version}")),
                ),
                PluginStatus::Invalid(reason) => ("Invalid", Some(reason)),
            };
            PluginRow {
                name: plugin.config.name,
                url: plugin.config.url.to_string(),
                version: plugin.manifest.map(|manifest| manifest.version),
                ok: status == "Running",
                status,
                detail,
            }
        })
        .collect();
    render(PluginsPageTemplate { plugins })
}

/// The htmx fragment for a plugin's tab: an iframe on the proxied entry page.
async fn plugin_page(State(state): State<AppState>, Path(name): Path<String>) -> Response {
    let tabs = state.plugins.ui_tabs().await;
    let Some(UiTab { title, entry, .. }) = tabs.into_iter().find(|tab| tab.name == name) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let ui_route = state.get_ui_route().await;
    render(PluginPageTemplate {
        title,
        src: format!("/{ui_route}/plugins/{name}/{entry}"),
    })
}

async fn proxy_root(state: State<AppState>, Path(name): Path<String>, req: Request) -> Response {
    proxy(state, Path((name, String::new())), req).await
}

/// Forward a request to the plugin, without the admin's cookies but with the
/// plugin's token. Responses can't set cookies on the admin origin.
async fn proxy(
    State(state): State<AppState>,
    Path((name, path)): Path<(String, String)>,
    req: Request,
) -> Response {
    let Some(target) = state.plugins.ui_target(&name).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let mut url = join_server_url(&target.url, &format!("/{path}"));
    url.set_query(req.uri().query());
    let prefix = format!("/{}/plugins/{name}", state.get_ui_route().await);

    let (parts, body) = req.into_parts();
    let body = match axum::body::to_bytes(body, MAX_BODY_BYTES).await {
        Ok(body) => body,
        Err(_) => return StatusCode::PAYLOAD_TOO_LARGE.into_response(),
    };
    let request = state
        .reqwest_client
        .request(parts.method, url)
        .headers(forwarded_request_headers(&parts.headers))
        .header("x-forwarded-prefix", prefix)
        .bearer_auth(target.token.as_str())
        .body(body);

    let response = match request.send().await {
        Ok(response) => response,
        Err(e) => {
            warn!("Plugin '{name}' UI request failed: {e}");
            return StatusCode::BAD_GATEWAY.into_response();
        }
    };
    let mut builder = Response::builder().status(response.status());
    for (key, value) in response.headers() {
        if !is_hop_by_hop(key) && key != header::SET_COOKIE {
            builder = builder.header(key, value);
        }
    }
    builder
        .body(Body::from_stream(response.bytes_stream()))
        .unwrap_or_else(|_| StatusCode::BAD_GATEWAY.into_response())
}

fn forwarded_request_headers(headers: &HeaderMap) -> HeaderMap {
    headers
        .iter()
        .filter(|(key, _)| {
            !is_hop_by_hop(key)
                && *key != header::COOKIE
                && *key != header::AUTHORIZATION
                && *key != header::HOST
                && *key != header::CONTENT_LENGTH
        })
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

fn is_hop_by_hop(name: &HeaderName) -> bool {
    matches!(
        name.as_str(),
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        encryption::Password,
        plugins::{test_support::state, PluginConfig},
    };
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    use wiremock::{
        matchers::{body_string, header as has_header, method, path, query_param},
        Mock, MockServer, ResponseTemplate,
    };

    async fn with_plugin(ui: bool) -> (AppState, MockServer) {
        let state = state().await;
        let server = MockServer::start().await;
        let mut manifest = serde_json::json!({
            "name": "viewer", "version": "0.1.0", "api_version": 1
        });
        if ui {
            manifest["ui"] = serde_json::json!({ "title": "Now playing", "entry": "index.html" });
        }
        Mock::given(method("GET"))
            .and(path("/manifest.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(manifest))
            .mount(&server)
            .await;
        state
            .plugins
            .load(&[PluginConfig {
                name: "viewer".to_string(),
                url: server.uri().parse().unwrap(),
                token: Password::from("ui-token"),
                enabled: true,
            }])
            .await;
        (state, server)
    }

    async fn send(state: &AppState, request: Request) -> (StatusCode, HeaderMap, String) {
        let response = router()
            .with_state(state.clone())
            .oneshot(request)
            .await
            .unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        (status, headers, String::from_utf8_lossy(&body).into_owned())
    }

    #[tokio::test]
    async fn proxies_pages_with_token_but_without_cookies() {
        let (state, server) = with_plugin(true).await;
        Mock::given(method("GET"))
            .and(path("/app/main.js"))
            .and(query_param("v", "2"))
            .and(has_header("authorization", "Bearer ui-token"))
            .and(has_header("x-forwarded-prefix", "/ui/plugins/viewer"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("set-cookie", "evil=1; Path=/")
                    .set_body_raw("console.log('hi')", "text/javascript"),
            )
            .mount(&server)
            .await;

        let (status, headers, body) = send(
            &state,
            Request::get("/plugins/viewer/app/main.js?v=2")
                .header("cookie", "id=admin-session")
                .header("authorization", "Bearer something-else")
                .body(Body::empty())
                .unwrap(),
        )
        .await;

        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body, "console.log('hi')");
        assert_eq!(headers["content-type"], "text/javascript");
        assert!(headers.get("set-cookie").is_none());
        let forwarded = server.received_requests().await.unwrap();
        let page_request = forwarded
            .iter()
            .find(|r| r.url.path() == "/app/main.js")
            .unwrap();
        assert!(page_request.headers.get("cookie").is_none());
    }

    #[tokio::test]
    async fn forwards_method_and_body() {
        let (state, server) = with_plugin(true).await;
        Mock::given(method("POST"))
            .and(path("/api/save"))
            .and(body_string("a=1"))
            .respond_with(ResponseTemplate::new(204))
            .mount(&server)
            .await;

        let (status, _, _) = send(
            &state,
            Request::post("/plugins/viewer/api/save")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from("a=1"))
                .unwrap(),
        )
        .await;

        assert_eq!(status, StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn refuses_plugins_without_ui_and_unknown_plugins() {
        let (state, _server) = with_plugin(false).await;

        for uri in [
            "/plugins/viewer/index.html",
            "/plugins/other/index.html",
            "/plugins/viewer",
        ] {
            let (status, _, _) = send(&state, Request::get(uri).body(Body::empty()).unwrap()).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{uri}");
        }
    }

    #[tokio::test]
    async fn tab_fragment_embeds_entry_page() {
        let (state, _server) = with_plugin(true).await;

        let (status, _, body) = send(
            &state,
            Request::get("/plugins/viewer").body(Body::empty()).unwrap(),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert!(
            body.contains(r#"src="/ui/plugins/viewer/index.html""#),
            "{body}"
        );
        assert!(body.contains("Now playing"), "{body}");
    }

    #[tokio::test]
    async fn status_page_lists_plugins() {
        let (state, _server) = with_plugin(true).await;

        let (status, _, body) = send(
            &state,
            Request::get("/plugins").body(Body::empty()).unwrap(),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("viewer") && body.contains("0.1.0") && body.contains("Running"));
        assert!(!body.contains("ui-token"), "token must never be shown");
    }
}
