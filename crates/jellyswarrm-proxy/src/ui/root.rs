use askama::Template;
use axum::{
    extract::State,
    http::StatusCode,
    response::{Html, IntoResponse},
};
use tracing::{error, info};

use crate::{
    ui::{
        auth::{AuthenticatedUser, UserRole},
        JellyfinUiVersion, JELLYFIN_UI_VERSION,
    },
    AppState,
};

#[derive(Template)]
#[template(path = "user/index.html")]
pub struct UserIndexTemplate {
    pub version: Option<String>,
    pub ui_route: String,
    pub root: Option<String>,
    pub jellyfin_ui_version: Option<JellyfinUiVersion>,
}

#[derive(Template)]
#[template(path = "admin/index.html")]
pub struct AdminIndexTemplate {
    pub version: Option<String>,
    pub ui_route: String,
    pub root: Option<String>,
    pub jellyfin_ui_version: Option<JellyfinUiVersion>,
    pub has_plugins: bool,
    pub plugin_tabs: Vec<crate::plugins::UiTab>,
}

/// Root/home page
pub async fn index(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
) -> impl IntoResponse {
    let response = if user.role == UserRole::User {
        let template = UserIndexTemplate {
            version: Some(env!("CARGO_PKG_VERSION").to_string()),
            ui_route: state.get_ui_route().await,
            root: state.get_url_prefix().await,
            jellyfin_ui_version: JELLYFIN_UI_VERSION.clone(),
        };

        match template.render() {
            Ok(html) => Html(html).into_response(),
            Err(e) => {
                error!("Failed to render index template: {}", e);
                (StatusCode::INTERNAL_SERVER_ERROR, "Template error").into_response()
            }
        }
    } else {
        info!("Rendering admin dashboard for {}", user.username);
        let template = AdminIndexTemplate {
            version: Some(env!("CARGO_PKG_VERSION").to_string()),
            ui_route: state.get_ui_route().await,
            root: state.get_url_prefix().await,
            jellyfin_ui_version: JELLYFIN_UI_VERSION.clone(),
            has_plugins: !state.plugins.plugins().await.is_empty(),
            plugin_tabs: state.plugins.ui_tabs().await,
        };

        match template.render() {
            Ok(html) => Html(html).into_response(),
            Err(e) => {
                error!("Failed to render index template: {}", e);
                (StatusCode::INTERNAL_SERVER_ERROR, "Template error").into_response()
            }
        }
    };
    response
}
