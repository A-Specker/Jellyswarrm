"""Jellyswarrm viewer plugin: shows who is watching what right now.

Follows the plugin event stream of Jellyswarrm and serves a page for the admin UI.
See docs/plugins/plugin-api-v1.md for the API this plugin uses.

Configuration (environment variables):
    PLUGIN_TOKEN         Token from this plugin's [[plugins]] entry (required).
    JELLYSWARRM_URL      Jellyswarrm as reachable from here (default http://localhost:3000).
    VIEWER_HOST          Listen address (default 0.0.0.0).
    VIEWER_PORT          Listen port (default 8765).
    STALE_AFTER_SECONDS  Treat a playback as ended after this long without a report
                         (default 60). Clients report about every 10 seconds, even
                         while paused, but send no stop when a browser tab is closed.
    VIEWER_API_KEYS      Comma-separated keys for the public API (one per app). Unset
                         or empty disables it. Keys need at least 16 characters.

Public API, for other apps (served directly by the viewer, not through Jellyswarrm):
    GET /public/v1/now-playing   with `Authorization: Bearer <key>` or `?api_key=<key>`.
"""

import hmac
import json
import logging
import os
import re
import threading
import time
from datetime import datetime, timezone
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, urlsplit
from urllib.request import Request, urlopen

VERSION = "0.2.0"
API_VERSION = 1
STATIC_DIR = Path(__file__).parent / "static"
PUBLIC_PATH = "/public/v1/now-playing"
MIN_API_KEY_LENGTH = 16

MANIFEST = {
    "name": "viewer",
    "version": VERSION,
    "api_version": API_VERSION,
    "ui": {"title": "Now playing", "icon": "fa-tv", "entry": "index.html"},
}

log = logging.getLogger("viewer")


def seconds_since(timestamp: str) -> float:
    """Age of an RFC 3339 timestamp from Jellyswarrm, in seconds."""
    then = datetime.fromisoformat(timestamp.replace("Z", "+00:00"))
    return (datetime.now(timezone.utc) - then).total_seconds()


class NowPlaying:
    """Current playbacks, built from plugin events and refreshed from /sessions."""

    def __init__(self, stale_after: float, clock=time.monotonic):
        self._lock = threading.Lock()
        self._stale_after = stale_after
        self._clock = clock
        # session_id -> latest playback report, with "seen" on our clock
        self._playing: dict[str, dict] = {}
        # session_id -> session from /plugin-api/v1/sessions (device, item names)
        self._sessions: dict[str, dict] = {}

    def apply_event(self, name: str, data: dict) -> bool:
        """Apply one event. Returns True when /sessions should be reloaded."""
        if name == "stream.lagged":
            return True
        if not name.startswith("playback."):
            return False
        session_id = data["session_id"]
        with self._lock:
            if name == "playback.stopped":
                self._playing.pop(session_id, None)
                return False
            self._playing[session_id] = {
                "user": data["user"],
                "item_id": data["item_id"],
                "server": data.get("server"),
                "position_ticks": data.get("position_ticks"),
                "is_paused": bool(data.get("is_paused")),
                "seen": self._clock(),
            }
            known = self._sessions.get(session_id)
        # A new item or session: fetch its title and device.
        return name == "playback.started" or known is None

    def apply_sessions(self, sessions: list) -> None:
        """Take over device info and titles, and playbacks we haven't seen events for."""
        now = self._clock()
        with self._lock:
            self._sessions = {session["id"]: session for session in sessions}
            for session in sessions:
                item = session.get("now_playing")
                if not item or session["id"] in self._playing:
                    continue
                age = seconds_since(session["last_activity"])
                if age < self._stale_after:
                    self._playing[session["id"]] = {
                        "user": session["user"],
                        "item_id": item["item_id"],
                        "server": item.get("server"),
                        "position_ticks": item.get("position_ticks"),
                        "is_paused": bool(item.get("is_paused")),
                        "seen": now - age,
                    }

    def snapshot(self) -> list:
        """Playbacks reported within the stale limit, for the page."""
        now = self._clock()
        result = []
        with self._lock:
            for session_id in list(self._playing):
                entry = self._playing[session_id]
                age = now - entry["seen"]
                if age > self._stale_after:
                    del self._playing[session_id]
                    continue
                session = self._sessions.get(session_id, {})
                item = session.get("now_playing") or {}
                if item.get("item_id") != entry["item_id"]:
                    item = {}
                device = session.get("device") or {}
                result.append(
                    {
                        "session_id": session_id,
                        "user": entry["user"]["name"],
                        "client": device.get("client"),
                        "device": device.get("name"),
                        "title": item.get("name"),
                        "series_name": item.get("series_name"),
                        "item_id": entry["item_id"],
                        "server": (entry["server"] or {}).get("name"),
                        "position_ticks": entry["position_ticks"],
                        "is_paused": entry["is_paused"],
                        "seconds_since_report": round(age),
                    }
                )
        result.sort(key=lambda row: (row["user"].lower(), row["session_id"]))
        return result


class Jellyswarrm:
    """Minimal client for the plugin API."""

    def __init__(self, base_url: str, token: str):
        self._base_url = base_url.rstrip("/")
        self._headers = {"Authorization": f"Bearer {token}"}

    def _open(self, path: str, timeout: float):
        request = Request(self._base_url + path, headers=self._headers)
        return urlopen(request, timeout=timeout)

    def sessions(self) -> list:
        with self._open("/plugin-api/v1/sessions", timeout=10) as response:
            return json.load(response)

    def events(self, on_connect):
        """Yield (event, data) from the event stream. Calls on_connect once connected,
        so state can be reloaded without missing events in between."""
        # Jellyswarrm sends keep-alive comments every 15 s; a silent minute means
        # the connection is dead.
        with self._open("/plugin-api/v1/events", timeout=60) as response:
            on_connect()
            yield from parse_sse(response)


def parse_sse(lines):
    """Parse a Server-Sent Events byte stream into (event, data) pairs."""
    event, data = "message", []
    for raw in lines:
        line = raw.decode("utf-8").rstrip("\r\n")
        if not line:
            if data:
                yield event, json.loads("\n".join(data))
            event, data = "message", []
        elif line.startswith(":"):
            continue
        elif line.startswith("event:"):
            event = line[len("event:"):].strip()
        elif line.startswith("data:"):
            data.append(line[len("data:"):].lstrip())


def follow_events(client: Jellyswarrm, state: NowPlaying, stop: threading.Event) -> None:
    """Keep `state` current; reconnects with backoff until `stop` is set."""

    def refresh():
        state.apply_sessions(client.sessions())

    backoff = 1
    while not stop.is_set():
        try:
            for name, data in client.events(on_connect=refresh):
                backoff = 1
                if state.apply_event(name, data):
                    refresh()
                if stop.is_set():
                    return
            log.warning("Event stream ended; reconnecting")
        except Exception as error:  # network errors, 401 while Jellyswarrm starts, ...
            log.warning("Event stream failed: %s; retrying in %ss", error, backoff)
            stop.wait(backoff)
            backoff = min(backoff * 2, 30)


def parse_api_keys(value: str | None, plugin_token: str) -> list[str]:
    """Keys for the public API from VIEWER_API_KEYS. Raises ValueError for unsafe keys."""
    keys = [key.strip() for key in (value or "").split(",") if key.strip()]
    for key in keys:
        if len(key) < MIN_API_KEY_LENGTH:
            raise ValueError(
                f"VIEWER_API_KEYS: every key needs at least {MIN_API_KEY_LENGTH} characters"
            )
        if hmac.compare_digest(key.encode(), plugin_token.encode()):
            raise ValueError("VIEWER_API_KEYS must not contain PLUGIN_TOKEN")
    return keys


def bearer(header: str | None) -> str:
    """The token of an `Authorization: Bearer <token>` header, or ""."""
    return header[len("Bearer "):] if header and header.startswith("Bearer ") else ""


def make_handler(state: NowPlaying, token: str, api_keys=()):
    expected = token.encode()
    keys = [key.encode() for key in api_keys]
    index = (STATIC_DIR / "index.html").read_bytes()

    def is_api_key(candidate: str) -> bool:
        # Compare against every key so the timing doesn't reveal which one matched.
        matched = False
        for key in keys:
            matched |= hmac.compare_digest(candidate.encode(), key)
        return matched

    class Handler(BaseHTTPRequestHandler):
        server_version = f"jellyswarrm-viewer/{VERSION}"

        def do_GET(self):
            url = urlsplit(self.path)
            if url.path == PUBLIC_PATH:
                return self._public(url.query)
            # Only Jellyswarrm knows the plugin token; it adds it when proxying admin requests.
            presented = bearer(self.headers.get("Authorization")).encode()
            if not hmac.compare_digest(presented, expected):
                return self._send(401, b"unauthorized", "text/plain")
            if url.path == "/manifest.json":
                self._send_json(MANIFEST)
            elif url.path in ("/", "/index.html"):
                self._send(200, index, "text/html; charset=utf-8")
            elif url.path == "/api/now-playing":
                self._send_json(state.snapshot())
            else:
                self._send(404, b"not found", "text/plain")

        def do_OPTIONS(self):
            # CORS preflight, so browser apps on other origins can send the key header.
            if urlsplit(self.path).path != PUBLIC_PATH or not keys:
                return self._send(404, b"not found", "text/plain")
            self.send_response(204)
            self.send_header("Access-Control-Allow-Origin", "*")
            self.send_header("Access-Control-Allow-Methods", "GET, OPTIONS")
            self.send_header("Access-Control-Allow-Headers", "Authorization")
            self.send_header("Access-Control-Max-Age", "86400")
            self.send_header("Content-Length", "0")
            self.end_headers()

        def _public(self, query: str):
            """The public API: other apps, authenticated with one of VIEWER_API_KEYS."""
            if not keys:
                return self._send(404, b"not found", "text/plain")
            candidate = bearer(self.headers.get("Authorization"))
            if not candidate:
                candidate = parse_qs(query).get("api_key", [""])[0]
            if not candidate or not is_api_key(candidate):
                return self._send(401, b"unauthorized", "text/plain", cors=True)
            self._send_json(state.snapshot(), cors=True)

        def _send_json(self, value, cors: bool = False):
            self._send(200, json.dumps(value).encode(), "application/json", cors)

        def _send(self, status: int, body: bytes, content_type: str, cors: bool = False):
            self.send_response(status)
            self.send_header("Content-Type", content_type)
            self.send_header("Content-Length", str(len(body)))
            self.send_header("Cache-Control", "no-store")
            if cors:
                self.send_header("Access-Control-Allow-Origin", "*")
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, format, *args):
            # Request lines can carry ?api_key=...; never log the key.
            message = re.sub(r"(api_key=)[^&\s]*", r"\1***", format % args)
            log.debug("%s - %s", self.address_string(), message)

    return Handler


def main():
    logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(message)s")
    token = os.environ.get("PLUGIN_TOKEN")
    if not token:
        raise SystemExit("PLUGIN_TOKEN is required (the token of this plugin's [[plugins]] entry)")
    base_url = os.environ.get("JELLYSWARRM_URL", "http://localhost:3000")
    host = os.environ.get("VIEWER_HOST", "0.0.0.0")
    port = int(os.environ.get("VIEWER_PORT", "8765"))
    stale_after = float(os.environ.get("STALE_AFTER_SECONDS", "60"))
    try:
        api_keys = parse_api_keys(os.environ.get("VIEWER_API_KEYS"), token)
    except ValueError as error:
        raise SystemExit(str(error))

    state = NowPlaying(stale_after)
    stop = threading.Event()
    threading.Thread(
        target=follow_events,
        args=(Jellyswarrm(base_url, token), state, stop),
        daemon=True,
    ).start()

    server = ThreadingHTTPServer((host, port), make_handler(state, token, api_keys))
    log.info("Viewer %s listening on %s:%s, following %s", VERSION, host, port, base_url)
    if api_keys:
        log.info("Public API enabled at %s for %d key(s)", PUBLIC_PATH, len(api_keys))
    else:
        log.info("Public API disabled (VIEWER_API_KEYS not set)")
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        stop.set()
        server.server_close()


if __name__ == "__main__":
    main()
