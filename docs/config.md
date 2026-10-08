# Jellyswarrm Configuration Documentation  

Jellyswarrm stores its configuration in a **TOML** file located at:  
`./data/jellyswarrm.toml` (inside the container).  

The SQLite database is stored at:  
`./data/jellyswarrm.db`.  

To persist your configuration and database across container restarts, mount a volume to the `./data` directory.  

You can override the default configuration in two ways:  
1. Provide your own `jellyswarrm.toml` file and mount it into the container.  
2. Use environment variables to override individual settings.  

---

## Configuration Options  

The table below lists all available configuration options:  

| Variable | Default Value | Environment Key | Description |
|----------|---------------|-----------------|-------------|
| `server_id` | *Generated UUID (32 hex chars)* | `JELLYSWARRM_SERVER_ID` | Unique identifier for the proxy server instance. |
| `public_address` | `localhost:3000` | `JELLYSWARRM_PUBLIC_ADDRESS` | Public address where the proxy is accessible. |
| `server_name` | `Jellyswarrm Proxy` | `JELLYSWARRM_SERVER_NAME` | Display name for the proxy server. |
| `host` | `0.0.0.0` | `JELLYSWARRM_HOST` | Host address the server binds to. |
| `port` | `3000` | `JELLYSWARRM_PORT` | Port number for the proxy server. |
| `include_server_name_in_media` | `true` | `JELLYSWARRM_INCLUDE_SERVER_NAME_IN_MEDIA` | Append the server name to media titles in responses. |
| `username` | `admin` | `JELLYSWARRM_USERNAME` | Default admin username. |
| `password` | `jellyswarrm` | `JELLYSWARRM_PASSWORD` | Default admin password (⚠️ change this in production). |
| `session_key` | *Generated 64-byte key* | `JELLYSWARRM_SESSION_KEY` | Base64-encoded session encryption key. |
| `timeout` | `20` | `JELLYSWARRM_TIMEOUT` | Request timeout in seconds. |
| `preconfigured_servers` | `[]` | `JELLYSWARRM_PRECONFIGURED_SERVERS` | Optional list of preconfigured Jellyfin servers (`url`, `name`, `priority`, `media_streaming_mode`). |
| `ui_route` | `ui` | `JELLYSWARRM_UI_ROUTE` | URL path segment for accessing the web UI (e.g., `/ui`). |
| `url_prefix` | *(none)* | `JELLYSWARRM_URL_PREFIX` | Optional URL prefix for all routes (useful for reverse proxy setups). |
| `server_background_check_interval_secs` | `30` | `JELLYSWARRM_SERVER_BACKGROUND_CHECK_INTERVAL_SECS` | Interval in seconds for background server health checks. |
| `auto_create_users_on_login` | `true` | `JELLYSWARRM_AUTO_CREATE_USERS_ON_LOGIN` | Automatically create local users on successful upstream login. |
| `merge_libraries` | `true` | `JELLYSWARRM_MERGE_LIBRARIES` | Merge libraries with matching names across servers into virtual libraries. |
| `deduplicate_media` | `false` | `JELLYSWARRM_DEDUPLICATE_MEDIA` | Collapse the same movie or show (series/season/episode) on multiple servers into one item whose versions are served by the different hosts (Jellyfin-style linked versions; Jellyfin v12 adds multi-versions for episodes). Legacy key `deduplicate_movies` / env `JELLYSWARRM_DEDUPLICATE_MOVIES` still loads. |
| `plugins` | `[]` | *(config file only)* | External plugin services (`name`, `url`, `token`, `enabled`). See [Plugins](#plugins). |

---

### Notes
- The `session_key` is generated as a secure 64-byte key if not specified, and is stored in the config file for reuse.  
- Each server now has its own streaming mode (`Redirect` or `Proxy`). For preconfigured servers, omit `media_streaming_mode` to use the default `Redirect`.
- Configuration files are resolved from the data directory (`./data` by default), which can be overridden with `JELLYSWARRM_DATA_DIR`.

---

## Plugins

Plugins are separate services that Jellyswarrm connects to; see the
[plugin docs](plugins/README.md). Each plugin is one `[[plugins]]` entry:

```toml
[[plugins]]
name = "viewer"                  # unique; lowercase letters, digits and '-'
url = "http://viewer:8765"       # as reachable from Jellyswarrm
token = "<random secret>"        # also configured in the plugin
enabled = true                   # optional, default true
```

| Field | Required | Description |
|---|---|---|
| `name` | yes | Unique name, used in URLs such as `/ui/plugins/{name}/`. Must match the `name` in the plugin's `manifest.json`. |
| `url` | yes | Base URL of the plugin service. Jellyswarrm loads `{url}/manifest.json` from it. |
| `token` | yes | Shared secret. The plugin sends it to the [plugin API](plugins/plugin-api-v1.md#authentication), and Jellyswarrm sends it to the plugin. Use a long random value, for example `openssl rand -base64 32`. |
| `enabled` | no | `false` keeps the entry but neither loads the plugin nor accepts its token. |

Changes take effect after a restart or after **Reload configuration** in the
admin settings. The **Plugins** tab in the admin UI shows each plugin's version
and status: `Running`, `Unreachable`, `Incompatible`, `Invalid` or `Disabled`.
An unreachable plugin never stops Jellyswarrm from starting.

The token is never shown in the admin UI or logs. Plugins with a page in the
admin UI run their scripts with the admin's browser session, so only configure
plugins you trust as much as Jellyswarrm itself.

> [!WARNING]
> Jellyswarrm builds without the plugin system, such as the official images,
> don't know `[[plugins]]` and drop the entries whenever they rewrite the
> configuration file. That happens when the file has no `session_key` yet, and
> when settings are saved in the admin UI. Keep a copy of your `[[plugins]]`
> entries if you switch between builds.
