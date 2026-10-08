# Viewer plugin

Shows who is watching what right now, across all Jellyswarrm backends, as a
**Now playing** tab in the Jellyswarrm admin UI.

It is an example of an external plugin (see [docs/plugins](../../docs/plugins/README.md)):
a small Python server without dependencies that follows the plugin event stream
and serves a page that Jellyswarrm shows inside the admin UI.

## How it works

* On start, and after every reconnect, it loads `GET /plugin-api/v1/sessions`.
* It follows `GET /plugin-api/v1/events` and updates the playbacks from
  `playback.started`, `playback.progress` and `playback.stopped`.
* A playback without a report for `STALE_AFTER_SECONDS` (default 60) counts as
  ended. Clients report about every 10 seconds, even while paused, but send no
  stop when a browser tab is closed.
* The page uses the admin UI's own styles (Pico CSS, Font Awesome) and follows
  its dark/light setting. It refreshes every 3 seconds.
* Every request needs the plugin token, which Jellyswarrm adds when it proxies
  the admin UI. Opening the plugin directly gives `401`.

## Configuration

Register the plugin in `jellyswarrm.toml`:

```toml
[[plugins]]
name = "viewer"
url = "http://viewer:8765"   # as reachable from Jellyswarrm
token = "<random secret>"
```

Then reload the configuration in the admin settings or restart Jellyswarrm.
Without access to the config file, use the `JELLYSWARRM_PLUGINS` environment
variable instead, see [Running](#running).

| Variable | Default | Meaning |
|---|---|---|
| `PLUGIN_TOKEN` | (required) | Same value as `token` above. |
| `JELLYSWARRM_URL` | `http://localhost:3000` | Jellyswarrm as reachable from the plugin, including `url_prefix` if set. |
| `VIEWER_HOST` | `0.0.0.0` | Listen address. |
| `VIEWER_PORT` | `8765` | Listen port. |
| `STALE_AFTER_SECONDS` | `60` | When a playback without reports counts as ended. |
| `VIEWER_API_KEYS` | *(unset)* | Comma-separated keys for the [public API](#public-api), one per app. Unset disables the public API. |

## Public API

Other apps (dashboards, e-ink displays, home automation, ...) can read the
current playbacks directly from the viewer, without going through Jellyswarrm:

```bash
curl -H "Authorization: Bearer <key>" http://viewer:8765/public/v1/now-playing
# or, for clients that can't set headers:
curl "http://viewer:8765/public/v1/now-playing?api_key=<key>"
```

The response is the same JSON the **Now playing** page uses, one object per
playback:

```json
[
  {
    "session_id": "ce4c1959bb1f4416b14a1403fbf971fb",
    "user": "test",
    "client": "Jellyfin Web",
    "device": "Edge Chromium",
    "title": "Big Buck Bunny",
    "series_name": null,
    "item_id": "cc62d033744d4a0594b09ef65ebf9793",
    "server": "Movies 1",
    "position_ticks": 281030780,
    "is_paused": true,
    "seconds_since_report": 4
  }
]
```

`position_ticks` are 100-nanosecond units (divide by 10,000,000 for seconds).
`title`, `series_name`, `client`, `device` and `server` can be `null`.

* **Keys:** set `VIEWER_API_KEYS` to one or more keys of at least 16
  characters, e.g. `openssl rand -base64 32`. Give each app its own key, so you
  can revoke one by removing it and restarting the viewer. Without keys the
  endpoint answers `404`.
* **Separate from the plugin token:** API keys only work on `/public/...`, and
  the plugin token doesn't work there. A leaked API key gives no access to
  Jellyswarrm.
* **Browsers:** the endpoint sends CORS headers, so web apps on other origins
  can call it.
* **Privacy:** the API shows who is watching what. The app must be able to
  reach the viewer's port; don't expose it to the internet without HTTPS and a
  reverse proxy in front of it.

## Running

With Docker Compose, next to Jellyswarrm, configured only through environment
variables. Put the secrets into a `.env` file next to the compose file:

```bash
# .env
VIEWER_TOKEN=<random secret, e.g. openssl rand -base64 32>
VIEWER_API_KEYS=<one key per app that uses the public API>
```

```yaml
services:
  jellyswarrm:
    # ... your Jellyswarrm build with the plugin system ...
    environment:
      - 'JELLYSWARRM_PLUGINS=[{"name":"viewer","url":"http://viewer:8765","token":"${VIEWER_TOKEN}"}]'

  viewer:
    build: plugins/viewer
    restart: unless-stopped
    environment:
      - PLUGIN_TOKEN=${VIEWER_TOKEN}
      - JELLYSWARRM_URL=http://jellyswarrm:3000
      - VIEWER_API_KEYS=${VIEWER_API_KEYS}
    # Only needed if apps outside this compose project use the public API:
    # ports:
    #   - "8765:8765"
```

For local development (Python 3.11 or newer), against the dev stack and a debug
build of Jellyswarrm, which reads the matching entry from
`data/jellyswarrm.dev.toml`:

```bash
PLUGIN_TOKEN=pj2_nIyzXXMEBmS26N-9zVw3D4o9ei_A python plugins/viewer/viewer.py
```

## Tests

```bash
cd plugins/viewer
python -m unittest -v
```
