# Plugin API

The plugin API lets external services ("plugins") read data from Jellyswarrm:
servers, users, active sessions and virtual libraries. A plugin is any HTTP
client, in any language, typically running in its own container next to
Jellyswarrm. Example uses: a "who is watching" dashboard, notifications, usage
statistics.

The API is **read-only** and **versioned**. Everything described here is
version 1 and lives under `/plugin-api/v1`.

## Registering a plugin

Add one `[[plugins]]` entry per plugin to `jellyswarrm.toml`:

```toml
[[plugins]]
name = "viewer"                          # a-z, 0-9 and '-', unique
token = "<random secret, 32+ characters>" # e.g. `openssl rand -hex 32`
scopes = ["sessions:read", "servers:read"]
enabled = true                           # optional, defaults to true
```

| Field | Required | Meaning |
|---|---|---|
| `name` | yes | Identifies the plugin in logs. |
| `token` | yes | Shared secret the plugin sends with every request. At least 32 characters and different for every plugin. |
| `scopes` | no | What the plugin may read, see [Scopes](#scopes). Defaults to none. |
| `enabled` | no | Set to `false` to block the plugin without removing it. |

Jellyswarrm refuses to start, and a reload fails, if a name is invalid or
duplicated, or a token is too short or shared between plugins.

Changes take effect without a restart: use **Reload From Disk** on the admin
settings page. Plugins are configured in the TOML file only, not through
environment variables.

## Authentication

Every request sends the plugin's token as a bearer token:

```sh
curl -H "Authorization: Bearer $TOKEN" http://jellyswarrm:3000/plugin-api/v1/sessions
```

| Status | When |
|---|---|
| `401 Unauthorized` | Token missing, unknown, or the plugin is disabled. |
| `403 Forbidden` | The token is valid but the plugin lacks the endpoint's scope. |

Error bodies are JSON: `{"error": "..."}`. Responses carry
`Cache-Control: no-store`.

Jellyfin user tokens and API keys are not accepted here, and plugin tokens are
not accepted by the Jellyfin API.

If Jellyswarrm runs with a `url_prefix`, the API is below it:
`/<url_prefix>/plugin-api/v1/...`.

## Scopes

| Scope | Endpoint |
|---|---|
| `servers:read` | `GET /plugin-api/v1/servers` |
| `users:read` | `GET /plugin-api/v1/users` |
| `sessions:read` | `GET /plugin-api/v1/sessions` |
| `libraries:read` | `GET /plugin-api/v1/libraries` |

Give each plugin only the scopes it needs.

## Endpoints

### `GET /plugin-api/v1/servers`

Backend servers, highest priority first.

```json
[
  { "id": 1, "name": "Movies", "priority": 100, "healthy": true }
]
```

`healthy` is the result of the latest background health check. Server URLs and
error details are not exposed.

### `GET /plugin-api/v1/users`

Jellyswarrm users.

```json
[
  { "id": "b077ff2c…", "name": "alice", "created_at": "2026-10-08T10:15:00Z" }
]
```

No passwords, tokens or backend account mappings are exposed.

### `GET /plugin-api/v1/sessions`

Active client sessions across all users, most recently active first.

```json
[
  {
    "id": "c0ffee…",
    "user": { "id": "b077ff2c…", "name": "alice" },
    "device": { "id": "e66e3c…", "name": "Living room TV", "client": "Jellyfin Android TV", "version": "0.19.10" },
    "last_activity": "2026-10-08T10:15:00Z",
    "now_playing": {
      "item_id": "6139f018…",
      "name": "Big Buck Bunny",
      "type": "Movie",
      "series_name": null,
      "position_ticks": 1234567890,
      "is_paused": false
    }
  }
]
```

* `now_playing` is `null` when nothing is playing.
* `item_id` is Jellyswarrm's virtual item ID, the same ID clients see.
* `name`, `type` and `series_name` are `null` until Jellyswarrm has served the
  item's metadata to that client.
* `position_ticks` is the last position the client reported (10,000,000 ticks =
  1 second), or `null` if it reported none.
* A session ends 30 minutes after its last activity, unless the client is still
  connected over WebSocket. Clients don't always report a stop (for example when
  a browser tab is closed), so treat a playback without progress reports for
  about a minute as ended. Clients report progress roughly every 10 seconds,
  even while paused.

### `GET /plugin-api/v1/libraries`

Virtual libraries and the backend libraries assigned to them. Empty when no
virtual libraries are configured.

```json
[
  {
    "id": "9f1c…",
    "name": "Movies",
    "collection_type": "movies",
    "members": [
      { "server": { "id": 1, "name": "Movies" }, "name": "Films" }
    ]
  }
]
```

`server.name` is `null` if the server was removed after the library was
assigned.

## Versioning

* Adding endpoints, scopes or fields is not a breaking change and stays in v1.
  Plugins must ignore fields they don't know.
* Removing or renaming anything requires `/plugin-api/v2`. Jellyswarrm can serve
  several versions at the same time.

## Security notes

* Treat plugin tokens like passwords. They are stored in plain text in
  `jellyswarrm.toml`. On Linux, Jellyswarrm writes that file readable by its
  owner only.
* Use HTTPS (for example through a reverse proxy) if plugins connect over an
  untrusted network.
* Every endpoint reads data that is normally visible to administrators only.
  Only register plugins you trust with that data.
