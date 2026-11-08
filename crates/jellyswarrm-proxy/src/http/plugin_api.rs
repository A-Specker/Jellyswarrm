//! Plugin API v1: read-only JSON for external plugins, authenticated with a
//! per-plugin bearer token and limited to the scopes in its registration.
//! Contract: `docs/plugin-api.md`.
use crate::{
    application::plugins::PluginQueries,
    config::plugins::{PluginIdentity, PluginScope},
    http::state::AppState,
};
use axum::{
    extract::State,
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde::Serialize;
use serde_json::json;

pub(crate) const PREFIX: &str = "/plugin-api/v1";

pub(crate) fn router() -> Router<AppState> {
    Router::new().nest(
        PREFIX,
        Router::new()
            .route("/servers", get(servers))
            .route("/users", get(users))
            .route("/sessions", get(sessions))
            .route("/libraries", get(libraries)),
    )
}

async fn servers(State(queries): State<PluginQueries>, headers: HeaderMap) -> Response {
    respond(&queries, &headers, PluginScope::ServersRead, async {
        queries.servers().await
    })
    .await
}

async fn users(State(queries): State<PluginQueries>, headers: HeaderMap) -> Response {
    respond(&queries, &headers, PluginScope::UsersRead, async {
        queries.users().await
    })
    .await
}

async fn sessions(State(queries): State<PluginQueries>, headers: HeaderMap) -> Response {
    respond(&queries, &headers, PluginScope::SessionsRead, async {
        Ok::<_, sqlx::Error>(queries.sessions().await)
    })
    .await
}

async fn libraries(State(queries): State<PluginQueries>, headers: HeaderMap) -> Response {
    respond(&queries, &headers, PluginScope::LibrariesRead, async {
        queries.libraries().await
    })
    .await
}

/// Authorize first, so unauthorized callers never trigger a query.
async fn respond<T: Serialize, E: std::fmt::Display>(
    queries: &PluginQueries,
    headers: &HeaderMap,
    scope: PluginScope,
    query: impl std::future::Future<Output = Result<T, E>>,
) -> Response {
    let plugin = match authorize(queries, headers, scope).await {
        Ok(plugin) => plugin,
        Err(denied) => return denied.into_response(),
    };
    let mut response = match query.await {
        Ok(body) => Json(body).into_response(),
        Err(failure) => {
            tracing::error!(plugin = %plugin.name, scope = scope.as_str(), "Plugin API query failed: {failure}");
            error(StatusCode::INTERNAL_SERVER_ERROR, "internal error")
        }
    };
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

/// Why a request was refused. Turned into a response only when needed.
enum Denied {
    Unauthenticated,
    MissingScope(PluginScope),
}

impl IntoResponse for Denied {
    fn into_response(self) -> Response {
        match self {
            Self::Unauthenticated => {
                let mut response =
                    error(StatusCode::UNAUTHORIZED, "missing or invalid plugin token");
                response
                    .headers_mut()
                    .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
                response
            }
            Self::MissingScope(scope) => error(
                StatusCode::FORBIDDEN,
                &format!("plugin lacks scope {}", scope.as_str()),
            ),
        }
    }
}

async fn authorize(
    queries: &PluginQueries,
    headers: &HeaderMap,
    scope: PluginScope,
) -> Result<PluginIdentity, Denied> {
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .ok_or(Denied::Unauthenticated)?;
    let plugin = queries
        .credentials
        .authenticate(token)
        .await
        .ok_or(Denied::Unauthenticated)?;
    if !plugin.allows(scope) {
        tracing::warn!(plugin = %plugin.name, scope = scope.as_str(), "Plugin API scope denied");
        return Err(Denied::MissingScope(scope));
    }
    Ok(plugin)
}

fn error(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "error": message }))).into_response()
}
