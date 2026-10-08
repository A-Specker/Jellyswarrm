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

## Step 2: Read API (done)

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

**Result:** verified against the dev stack. Known issue: `now_playing.name` and
`type` can show the media source instead of the item (e.g. `"Big Buck Bunny
(2008)"` / `"Default"`). The cause is the existing metadata cache
(`ClientSessionService::cache_media_response`), which also feeds Jellyswarrm's own
`/Sessions`; see Step 6.

## Step 3: Events (done, live stop not yet observed)

* `PluginEvent` enum and a `tokio::sync::broadcast` channel in the
  `PluginManager`.
* Publish from `sessions::observe_playback()` in `src/sessions/mod.rs`, the
  single place that already sees every successful playback report.
* `GET /plugin-api/v1/events` as Server-Sent Events, with keep-alive.

**Done when:** `curl -N` on the events endpoint prints `playback.started`,
`playback.progress` and `playback.stopped` while a video plays in the dev stack,
and a slow subscriber never delays playback requests.

**Result:** `playback.started` and `playback.progress` (including pause) verified
live with the web client. `playback.stopped` is covered by unit tests but wasn't
observed live, because the test ended by closing the tab.

Finding: **clients don't always report a stop.** Pausing keeps sending
`playback.progress` with `is_paused: true` about every 10 seconds. Closing the
browser tab sends no stop at all, neither to Jellyswarrm nor to the backend; the
session stays "playing" until it expires 30 minutes after its last activity.
Plugins must not rely on `playback.stopped` alone (see Steps 5 and 6).

## Step 4: Admin UI (done)

* "Plugins" tab in the admin UI listing each plugin with name, version and status.
* Reverse proxy `/{ui_route}/plugins/{name}/{*path}` behind the existing
  `require_admin` middleware in `src/ui/mod.rs`; strips the session cookie, adds
  the plugin token.
* One tab per plugin with a `ui` block, rendered in `templates/admin/index.html`
  via `AdminIndexTemplate` (`src/ui/root.rs`).

**Done when:** an admin sees the plugin's page inside the admin UI; a non-admin
gets `403`.

## Step 5: Viewer plugin (done)

* Separate directory or repository, its own container.
* Shows who is watching what right now, using `/sessions` on load and
  `/events` for live updates.
* The language is decided in this step.
* Treat a playback as ended when no `playback.progress` arrived for about 60
  seconds. Clients report every ~10 seconds even while paused, so silence means
  the client is gone (see the Step 3 finding).

**Done when:** the viewer shows live playback from the dev stack inside the admin
UI.

**Result:** `plugins/viewer/`, Python without dependencies, verified live in the
dev stack. Its page reuses the admin UI's Pico CSS and Font Awesome from
`/{ui_route}/resources/` and follows the dark/light setting.

## Step 6: Hardening

* Unit tests for the manager, API and auth, using `wiremock` like the existing
  tests.
* A dev-stack scenario with the viewer plugin.
* Document `[[plugins]]` in `docs/config.md`, including that builds without
  the plugin system drop `[[plugins]]` when they rewrite the config.
* Emit `playback.stopped` (or a new `session.ended`) when a session expires or
  its WebSocket disconnects, so plugins learn about clients that vanish without a
  stop report. Needs one more touch point in `src/sessions/service.rs`.
* Fix the metadata cache so media sources don't overwrite item metadata. This
  is an upstream bug, best sent as a separate PR rather than kept in the fork.

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
