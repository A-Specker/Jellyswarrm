# Plugin roadmap

Each step is small enough to review on its own and leaves Jellyswarrm working.
Code lives in `crates/jellyswarrm-proxy/src/plugins/` unless a step says
otherwise. Core changes are limited to the touch points listed at the end and
follow [0007 Fork hygiene](decisions/0007-fork-hygiene.md).

## Step 0: Design docs (done)

Goals, decisions and the API contract in `docs/plugins/`.

**Done when:** the docs are reviewed and agreed.

## Step 1: Skeleton (done)

* `src/plugins/{mod.rs, config.rs, manager.rs}`.
* `PluginConfig` (`name`, `url`, `token`, `enabled`) and a `plugins` list in
  `AppConfig`, modelled after `PreconfiguredServer` in `src/config.rs`.
* `PluginManager` stored in `AppState` (`src/main.rs`).
* On startup and on config reload (`reload_config` in
  `src/ui/admin/settings.rs`): fetch each enabled plugin's `manifest.json`,
  check `api_version`, record the status (`ok`, `unreachable`,
  `incompatible`) and log it.

**Done when:** a configured plugin's manifest is loaded and its status is logged;
without `[[plugins]]`, Jellyswarrm behaves exactly as before.

## Step 2: Read API (implemented, dev-stack check open)

* `plugins::router()` merged into the app, like `health::router()` in
  `src/main.rs`.
* Token middleware: `Authorization: Bearer <token>`, constant-time comparison,
  `401` otherwise.
* `GET /plugin-api/v1/sessions`, `/users`, `/servers` with their own DTO structs.
* Core additions:
  * `ClientSessionService::all()` in `src/sessions/service.rs` (today there is
    only `for_user()`).
  * Reuse `UserAuthorizationService::list_users()` and
    `ServerStorageService::list_servers()`.

**Done when:** `curl -H "Authorization: Bearer …" /plugin-api/v1/sessions` shows a
session that is playing in the dev stack, and requests without a token get `401`.

## Step 3: Events (implemented, dev-stack check open)

* `PluginEvent` enum and a `tokio::sync::broadcast` channel in the
  `PluginManager`.
* Publish from `sessions::observe_playback()` in `src/sessions/mod.rs`, the
  single place that already sees every successful playback report.
* `GET /plugin-api/v1/events` as Server-Sent Events, with keep-alive.

**Done when:** `curl -N` on the events endpoint prints `playback.started`,
`playback.progress` and `playback.stopped` while a video plays in the dev stack,
and a slow subscriber never delays playback requests.

## Step 4: Admin UI

* "Plugins" tab in the admin UI listing each plugin with name, version and status.
* Reverse proxy `/{ui_route}/plugins/{name}/{*path}` behind the existing
  `require_admin` middleware in `src/ui/mod.rs`; strips the session cookie, adds
  the plugin token.
* One tab per plugin with a `ui` block, rendered in `templates/admin/index.html`
  via `AdminIndexTemplate` (`src/ui/root.rs`).

**Done when:** an admin sees the plugin's page inside the admin UI; a non-admin
gets `403`.

## Step 5: Viewer plugin

* Separate directory or repository, its own container.
* Shows who is watching what right now, using `/sessions` on load and
  `/events` for live updates.
* The language is decided in this step.

**Done when:** the viewer shows live playback from the dev stack inside the admin
UI.

## Step 6: Hardening

* Unit tests for the manager, API and auth, using `wiremock` like the existing
  tests.
* A dev-stack scenario with the viewer plugin.
* Document `[[plugins]]` in `docs/config.md`.

**Done when:** `cargo test` covers the plugin module and the docs describe how to
run a plugin.

## Core touch points

| File | Change | Step |
|---|---|---|
| `src/main.rs` | `mod plugins;`, `plugins` field in `AppState`, merge `plugins::router()` | 1, 2 |
| `src/config.rs` | `plugins: Vec<PluginConfig>` in `AppConfig` | 1 |
| `src/ui/admin/settings.rs` | reload plugins in `reload_config` | 1 |
| `src/sessions/service.rs` | `ClientSessionService::all()` | 2 |
| `src/sessions/mod.rs` | publish playback events in `observe_playback()` | 3 |
| `src/ui/mod.rs`, `src/ui/root.rs`, `templates/admin/index.html` | plugin UI route and tabs | 4 |
