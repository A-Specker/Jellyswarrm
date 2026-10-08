//! Fixtures shared by the plugin module tests.
use std::sync::Arc;

use crate::{
    config::{AppConfig, MediaStreamingMode, MIGRATOR},
    encryption::Password,
    handlers::quick_connect::QuickConnectStorage,
    media_storage_service::MediaStorageService,
    plugins::PluginConfig,
    server_id::ServerId,
    server_storage::ServerStorageService,
    session_storage::SessionStorage,
    user_authorization_service::UserAuthorizationService,
    virtual_library_service::VirtualLibraryService,
    AppState, DataContext, ProxyProcessors,
};

/// Token of the "viewer" plugin registered by [`state`].
pub const TOKEN: &str = "plugin-token";

/// App state on an in-memory database with one registered plugin, "viewer".
pub async fn state() -> AppState {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    MIGRATOR.run(&pool).await.unwrap();
    let servers = ServerStorageService::new(pool.clone());
    let media = MediaStorageService::new(pool.clone());
    let data = DataContext {
        user_authorization: Arc::new(UserAuthorizationService::new(pool.clone())),
        server_storage: Arc::new(servers.clone()),
        media_storage: Arc::new(media.clone()),
        virtual_library_service: Arc::new(VirtualLibraryService::new(pool, servers, media)),
        play_sessions: Arc::new(SessionStorage::new()),
        config: Arc::new(tokio::sync::RwLock::new(AppConfig::default())),
    };
    let state = AppState::new(
        reqwest::Client::new(),
        reqwest::Client::new(),
        data.clone(),
        ProxyProcessors::new(data),
        QuickConnectStorage::new(),
    );
    // A plugin whose manifest cannot be fetched still authenticates.
    state
        .plugins
        .load(&[PluginConfig {
            name: "viewer".to_string(),
            url: "http://127.0.0.1:9".parse().unwrap(),
            token: Password::from(TOKEN),
            enabled: true,
        }])
        .await;
    state
}

/// A "Movies 1" server with one item; returns the server ID and the item's virtual ID.
pub async fn movie(state: &AppState) -> (ServerId, String) {
    let server_id = state
        .server_storage
        .add_server(
            "Movies 1",
            "http://127.0.0.1:8096",
            100,
            MediaStreamingMode::Proxy,
        )
        .await
        .unwrap();
    let server = state
        .server_storage
        .get_server_by_id(server_id)
        .await
        .unwrap()
        .unwrap();
    let item = state
        .media_storage
        .get_or_create_media_mapping("8db8e93fd3984ddd8c3a1410351e64d8", &server)
        .await
        .unwrap()
        .virtual_media_id;
    (server_id, item)
}
