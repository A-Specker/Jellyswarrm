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

| Variable | Default | Meaning |
|---|---|---|
| `PLUGIN_TOKEN` | (required) | Same value as `token` above. |
| `JELLYSWARRM_URL` | `http://localhost:3000` | Jellyswarrm as reachable from the plugin, including `url_prefix` if set. |
| `VIEWER_HOST` | `0.0.0.0` | Listen address. |
| `VIEWER_PORT` | `8765` | Listen port. |
| `STALE_AFTER_SECONDS` | `60` | When a playback without reports counts as ended. |

## Running

With Docker Compose, next to Jellyswarrm:

```yaml
services:
  viewer:
    build: plugins/viewer
    restart: unless-stopped
    environment:
      - PLUGIN_TOKEN=<random secret>
      - JELLYSWARRM_URL=http://jellyswarrm:3000
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
