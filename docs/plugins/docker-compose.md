# Deploying plugins with Docker Compose

This guide sets up Jellyswarrm with the [viewer plugin](../../plugins/viewer/README.md)
using only a compose file and a `.env` file. No `jellyswarrm.toml` needs to be
edited.

> [!IMPORTANT]
> The official image `ghcr.io/llukas22/jellyswarrm` does **not** contain the
> plugin system. Build Jellyswarrm from the `plugin-manager` branch as shown
> below. The official image ignores `JELLYSWARRM_PLUGINS` and removes plugin
> entries when it rewrites the config file.

## 1. Folder layout

Keep your compose file, secrets and data next to the source, not inside it, so
`git pull` never touches them:

```text
jellyswarrm-deploy/
├── Jellyswarrm/          # the source, cloned in step 2
├── docker-compose.yml    # step 4
├── .env                  # step 3
└── data/                 # created by Jellyswarrm
```

## 2. Get the source

```bash
mkdir jellyswarrm-deploy && cd jellyswarrm-deploy
git clone --recurse-submodules --branch plugin-manager https://github.com/A-Specker/Jellyswarrm.git
```

`--recurse-submodules` is required: the image build needs the Jellyfin web
client from the `ui` submodule.

Build on Linux (or WSL). On Windows, Git may convert line endings, which changes
the checksums of the database migrations; the image then refuses a database
created by another build.

## 3. Secrets

Create `.env` next to the compose file:

```bash
ADMIN_PASSWORD=<choose a password>
VIEWER_TOKEN=<random secret>
VIEWER_API_KEYS=<random secret>
```

Generate the random values with `openssl rand -base64 32`, or
`python3 -c "import secrets; print(secrets.token_urlsafe(32))"`.

* `VIEWER_TOKEN` connects Jellyswarrm and the viewer. Both services read it from
  `.env`, so it only exists once.
* `VIEWER_API_KEYS` is for other apps that read the viewer's
  [public API](../../plugins/viewer/README.md#public-api). Separate several keys
  with commas, one per app. Leave it empty to turn the public API off.

## 4. Compose file

`docker-compose.yml`:

```yaml
services:
  jellyswarrm:
    build: ./Jellyswarrm
    image: jellyswarrm:plugin-manager
    container_name: jellyswarrm
    restart: unless-stopped
    ports:
      - "3000:3000"
    volumes:
      - ./data:/app/data
    environment:
      - JELLYSWARRM_USERNAME=admin
      - JELLYSWARRM_PASSWORD=${ADMIN_PASSWORD:?set ADMIN_PASSWORD in .env}
      - 'JELLYSWARRM_PLUGINS=[{"name":"viewer","url":"http://viewer:8765","token":"${VIEWER_TOKEN:?set VIEWER_TOKEN in .env}"}]'
    depends_on:
      viewer:
        condition: service_healthy

  viewer:
    build: ./Jellyswarrm/plugins/viewer
    image: jellyswarrm-viewer:plugin-manager
    container_name: jellyswarrm-viewer
    restart: unless-stopped
    environment:
      - PLUGIN_TOKEN=${VIEWER_TOKEN:?set VIEWER_TOKEN in .env}
      - JELLYSWARRM_URL=http://jellyswarrm:3000
      - VIEWER_API_KEYS=${VIEWER_API_KEYS:-}
    healthcheck:
      test:
        - CMD-SHELL
        - 'wget -q -O /dev/null --header "Authorization: Bearer $$PLUGIN_TOKEN" http://127.0.0.1:8765/manifest.json || exit 1'
      start_period: 5s
      interval: 30s
      timeout: 3s
      retries: 3
    # Only for apps outside this compose project that use the public API:
    # ports:
    #   - "8765:8765"
```

Why it looks like this:

* **`JELLYSWARRM_PLUGINS`** registers the viewer. Keep the single quotes around
  the whole entry, so YAML leaves the JSON alone. See
  [configuration](../config.md#without-a-config-file-jellyswarrm_plugins).
* **`depends_on` with `service_healthy`**: Jellyswarrm loads plugin manifests at
  startup. If the viewer isn't up yet, it shows as *Unreachable* and its tab is
  missing until the next restart or **Reload configuration**.
* **The viewer has no `ports`** by default. Jellyswarrm reaches it inside the
  compose network, and admins use it through the Jellyswarrm admin UI. Publish
  port 8765 only for the public API, and only behind HTTPS if it leaves your
  home network: it shows who is watching what.
* The healthcheck uses `127.0.0.1` because `localhost` resolves to IPv6 inside
  the container, while the viewer listens on IPv4.

## 5. Start

```bash
docker compose up -d --build
```

The first build takes several minutes (Jellyfin web client and a Rust release
build). Then:

1. Open `http://<host>:3000/ui` and log in as `admin` with `ADMIN_PASSWORD`.
2. Add your Jellyfin servers under **Servers**, as usual.
3. The **Plugins** tab should list `viewer` as **Running**, and a
   **Now playing** tab should appear.
4. Play something in a Jellyfin client connected to Jellyswarrm. It shows up in
   **Now playing** within a few seconds.

Test the public API from another machine (with the port published):

```bash
curl -H "Authorization: Bearer <one of VIEWER_API_KEYS>" http://<host>:8765/public/v1/now-playing
```

## Updating

```bash
cd Jellyswarrm
git pull --recurse-submodules
cd ..
docker compose up -d --build
```

Changes to `.env` or the compose file also need `docker compose up -d`.
**Reload configuration** in the admin UI doesn't see new environment values.

## Troubleshooting

| Symptom | Cause and fix |
|---|---|
| No **Plugins** tab | The image has no plugin system (official image), or `JELLYSWARRM_PLUGINS` is empty. Check `docker compose logs jellyswarrm \| grep -i plugin`. |
| Jellyswarrm exits with `JELLYSWARRM_PLUGINS must be a JSON list` | The JSON is broken, usually missing quotes. Run `docker compose config` and compare the value with the example above. |
| Viewer is **Unreachable** | Jellyswarrm started before the viewer, or the `url` is wrong. Check `docker compose ps`, then use **Reload configuration**. |
| Viewer is **Invalid** | The `name` in `JELLYSWARRM_PLUGINS` doesn't match the plugin's manifest (`viewer`), or names repeat. |
| **Now playing** says *Could not load playbacks: HTTP 502* | Jellyswarrm can't reach the viewer. Check `docker compose logs viewer`. |
| Viewer logs `Event stream failed: HTTP Error 401` | `PLUGIN_TOKEN` and the token in `JELLYSWARRM_PLUGINS` differ. Both must use `${VIEWER_TOKEN}`. Right after startup, a few connection errors or 401s are normal: the viewer starts first and retries until Jellyswarrm has loaded its plugins. |
| Public API answers `404` | `VIEWER_API_KEYS` is empty, so the public API is off. |
| Public API answers `401` | Wrong key, or the plugin token was used instead of an API key. |
| Jellyswarrm exits with `migration ... was previously applied but has been modified` | The image was built from a checkout with Windows line endings. Rebuild on Linux or WSL; your data is unchanged. |
