# Plugin API v1

This is the contract between Jellyswarrm and a plugin. A plugin that follows it
keeps working across Jellyswarrm updates as long as the API version stays `1`.

> Status: design. Field lists may still change before the first implementation.

## Registration

An admin registers a plugin in `jellyswarrm.toml`:

```toml
[[plugins]]
name = "viewer"                  # unique, used in URLs: [a-z0-9-]
url = "http://viewer:8080"       # base URL, reachable from Jellyswarrm
token = "<random secret>"        # shared secret, see Authentication
enabled = true
```

## Manifest

Every plugin serves `GET {url}/manifest.json`:

```json
{
  "name": "viewer",
  "version": "0.1.0",
  "api_version": 1,
  "ui": {
    "title": "Now playing",
    "icon": "fa-tv",
    "entry": "index.html"
  }
}
```

| Field | Required | Meaning |
|---|---|---|
| `name` | yes | Must match `name` in the config. |
| `version` | yes | The plugin's own version, shown in the admin UI. |
| `api_version` | yes | Plugin API major version the plugin was built for. Jellyswarrm disables plugins with an unsupported version. |
| `ui` | no | Present if the plugin adds an admin page. |
| `ui.title` | yes, if `ui` | Tab label in the admin UI. |
| `ui.icon` | no | Font Awesome icon class, as used by the existing admin tabs. |
| `ui.entry` | yes, if `ui` | Path of the start page, relative to `url`. |

## Authentication

Each plugin has one token, configured in `[[plugins]]`. It is used in both
directions:

* **Plugin → Jellyswarrm:** every request to the plugin API sends
  `Authorization: Bearer <token>`. Requests without a valid token get `401`.
* **Jellyswarrm → plugin:** manifest and UI requests carry the same header, so a
  plugin can reject requests that don't come from Jellyswarrm.

## Endpoints

All endpoints are read-only, return JSON, and live under the root of
Jellyswarrm (respecting `url_prefix` if one is configured).

### `GET /plugin-api/v1/sessions`

Active client sessions across all users.

```json
[
  {
    "id": "c0ffee...",
    "user": { "id": "b077ff2c...", "name": "test" },
    "device": { "client": "Jellyfin Android TV", "name": "Onn-fassbar", "id": "e66e3c...", "version": "0.19.10" },
    "last_activity": "2026-10-08T10:15:00Z",
    "now_playing": {
      "item_id": "6139f018...",
      "name": "Big Buck Bunny",
      "type": "Movie",
      "server": { "id": "1", "name": "Movies 1" },
      "position_ticks": 1234567890,
      "is_paused": false
    }
  }
]
```

`now_playing` is `null` when the session is idle. Item IDs are Jellyswarrm's
virtual IDs, the same IDs clients see.

### `GET /plugin-api/v1/users`

```json
[ { "id": "b077ff2c...", "name": "test" } ]
```

No passwords, tokens or server mappings are exposed.

### `GET /plugin-api/v1/servers`

```json
[ { "id": "1", "name": "Movies 1", "healthy": true } ]
```

Server URLs are not exposed.

### `GET /plugin-api/v1/events`

A Server-Sent Events stream. Each event has an `event:` type and a JSON `data:`
payload:

```text
event: playback.started
data: {"session_id":"c0ffee...","user":{"id":"b077ff2c...","name":"test"},"item_id":"6139f018...","server":{"id":"1","name":"Movies 1"},"at":"2026-10-08T10:15:00Z"}
```

| Event | Sent when |
|---|---|
| `playback.started` | A client reports that playback started. |
| `playback.progress` | A client reports progress (typically every few seconds). Includes `position_ticks` and `is_paused`. |
| `playback.stopped` | A client reports that playback stopped. |

Delivery is **best effort**. If a plugin reads too slowly or is disconnected, it
misses events. Plugins that need a consistent picture combine the stream with
`GET /plugin-api/v1/sessions`, for example by reloading the session list after
reconnecting.

## UI pages

If the manifest has a `ui` block, Jellyswarrm:

* adds a tab with `ui.title` to the admin UI, and
* proxies `/{ui_route}/plugins/{name}/{path}` to `{url}/{path}`.

Rules for plugin pages:

* Only logged-in Jellyswarrm admins can reach them.
* Use **relative URLs** for assets and API calls, because pages are served below
  `/{ui_route}/plugins/{name}/`.
* The admin's Jellyswarrm session cookie is **not** forwarded. A plugin page that
  needs data calls its own backend, which calls the plugin API with its token.

## Versioning

* Adding endpoints, events or fields is **not** a breaking change and stays in
  v1. Plugins must ignore unknown fields and events.
* Removing or renaming anything is a breaking change and requires
  `/plugin-api/v2` and `api_version: 2`. Jellyswarrm may support several versions
  at the same time.
