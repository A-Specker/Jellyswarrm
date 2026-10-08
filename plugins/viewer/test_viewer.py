"""Tests for the viewer plugin. Run with: python -m unittest -v (in plugins/viewer)."""

import io
import json
import threading
import unittest
from datetime import datetime, timedelta, timezone
from http.server import ThreadingHTTPServer
from urllib.error import HTTPError
from urllib.request import Request, urlopen

from viewer import MANIFEST, NowPlaying, make_handler, parse_api_keys, parse_sse


class FakeClock:
    def __init__(self):
        self.now = 1000.0

    def __call__(self):
        return self.now


def playback(event_session="s1", item="item-1", position=10, paused=False):
    return {
        "session_id": event_session,
        "user": {"id": "u1", "name": "alice"},
        "item_id": item,
        "server": {"id": "1", "name": "Movies 1"},
        "position_ticks": position,
        "is_paused": paused,
        "at": "2026-10-08T12:00:00Z",
    }


def session(session_id="s1", item="item-1", age_seconds=0.0):
    last_activity = datetime.now(timezone.utc) - timedelta(seconds=age_seconds)
    return {
        "id": session_id,
        "user": {"id": "u1", "name": "alice"},
        "device": {"client": "Jellyfin Web", "name": "Edge", "id": "d1", "version": "12"},
        # Jellyswarrm sends nanosecond precision.
        "last_activity": last_activity.strftime("%Y-%m-%dT%H:%M:%S.%f") + "500Z",
        "now_playing": None
        if item is None
        else {
            "item_id": item,
            "name": "Big Buck Bunny",
            "type": "Movie",
            "series_name": None,
            "server": {"id": "1", "name": "Movies 1"},
            "position_ticks": 5,
            "is_paused": True,
        },
    }


class NowPlayingTest(unittest.TestCase):
    def setUp(self):
        self.clock = FakeClock()
        self.state = NowPlaying(stale_after=60, clock=self.clock)

    def test_started_and_progress_events_show_playback(self):
        self.assertTrue(self.state.apply_event("playback.started", playback()))
        self.state.apply_sessions([session()])
        self.state.apply_event("playback.progress", playback(position=20, paused=True))

        [row] = self.state.snapshot()
        self.assertEqual(row["user"], "alice")
        self.assertEqual(row["title"], "Big Buck Bunny")
        self.assertEqual(row["device"], "Edge")
        self.assertEqual(row["server"], "Movies 1")
        self.assertEqual(row["position_ticks"], 20)
        self.assertTrue(row["is_paused"])

    def test_stopped_event_removes_playback(self):
        self.state.apply_event("playback.started", playback())
        self.state.apply_event("playback.stopped", playback())

        self.assertEqual(self.state.snapshot(), [])

    def test_playback_without_reports_expires(self):
        self.state.apply_event("playback.progress", playback())
        self.clock.now += 59
        self.assertEqual(len(self.state.snapshot()), 1)

        self.clock.now += 2
        self.assertEqual(self.state.snapshot(), [])

    def test_title_is_hidden_when_session_shows_another_item(self):
        self.state.apply_sessions([session(item="old-item")])
        self.state.apply_event("playback.started", playback(item="new-item"))

        [row] = self.state.snapshot()
        self.assertIsNone(row["title"])
        self.assertEqual(row["item_id"], "new-item")

    def test_sessions_add_recent_playbacks_only(self):
        self.state.apply_sessions(
            [
                session("recent", age_seconds=5),
                session("old", age_seconds=600),
                session("idle", item=None),
            ]
        )

        rows = self.state.snapshot()
        self.assertEqual([row["session_id"] for row in rows], ["recent"])
        self.assertEqual(rows[0]["seconds_since_report"], 5)

    def test_lag_requests_refresh_and_unknown_events_are_ignored(self):
        self.assertTrue(self.state.apply_event("stream.lagged", {"missed": 3}))
        self.assertFalse(self.state.apply_event("login.something", {}))
        self.assertEqual(self.state.snapshot(), [])

    def test_progress_for_known_session_needs_no_refresh(self):
        self.state.apply_sessions([session()])
        self.assertFalse(self.state.apply_event("playback.progress", playback()))


class ParseSseTest(unittest.TestCase):
    def test_parses_events_and_skips_comments(self):
        stream = io.BytesIO(
            b": keep-alive\n\n"
            b"event: playback.started\n"
            b'data: {"session_id": "s1"}\n\n'
            b"event: stream.lagged\r\n"
            b'data: {"missed": 2}\r\n\r\n'
        )

        self.assertEqual(
            list(parse_sse(stream)),
            [("playback.started", {"session_id": "s1"}), ("stream.lagged", {"missed": 2})],
        )


class HandlerTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.state = NowPlaying(stale_after=60)
        cls.server = ThreadingHTTPServer(("127.0.0.1", 0), make_handler(cls.state, "secret"))
        threading.Thread(target=cls.server.serve_forever, daemon=True).start()
        cls.base = f"http://127.0.0.1:{cls.server.server_address[1]}"

    @classmethod
    def tearDownClass(cls):
        cls.server.shutdown()
        cls.server.server_close()

    def get(self, path, token="secret"):
        headers = {"Authorization": f"Bearer {token}"} if token else {}
        with urlopen(Request(self.base + path, headers=headers), timeout=5) as response:
            return response.status, response.headers["Content-Type"], response.read()

    def test_requires_token(self):
        for token in (None, "wrong"):
            with self.assertRaises(HTTPError) as error:
                self.get("/manifest.json", token=token)
            self.assertEqual(error.exception.code, 401)

    def test_serves_manifest_page_and_data(self):
        _, _, manifest = self.get("/manifest.json")
        self.assertEqual(json.loads(manifest), MANIFEST)

        status, content_type, page = self.get("/index.html")
        self.assertEqual(status, 200)
        self.assertIn("text/html", content_type)
        self.assertIn(b"Now playing", page)

        _, content_type, data = self.get("/api/now-playing?x=1")
        self.assertEqual(content_type, "application/json")
        self.assertEqual(json.loads(data), [])

    def test_unknown_paths_are_not_found(self):
        with self.assertRaises(HTTPError) as error:
            self.get("/../viewer.py")
        self.assertEqual(error.exception.code, 404)

    def test_public_api_is_off_without_keys(self):
        for token in ("secret", None):
            with self.assertRaises(HTTPError) as error:
                self.get("/public/v1/now-playing", token=token)
            self.assertEqual(error.exception.code, 404)


APP_KEY = "app-key-0123456789"


class PublicApiTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.state = NowPlaying(stale_after=60)
        cls.state.apply_event("playback.started", playback())
        cls.server = ThreadingHTTPServer(
            ("127.0.0.1", 0), make_handler(cls.state, "secret", [APP_KEY, "other-app-key-0123"])
        )
        threading.Thread(target=cls.server.serve_forever, daemon=True).start()
        cls.base = f"http://127.0.0.1:{cls.server.server_address[1]}"

    @classmethod
    def tearDownClass(cls):
        cls.server.shutdown()
        cls.server.server_close()

    def request(self, path, token=None, method="GET"):
        headers = {"Authorization": f"Bearer {token}"} if token else {}
        request = Request(self.base + path, headers=headers, method=method)
        with urlopen(request, timeout=5) as response:
            return response.status, response.headers, response.read()

    def assert_status(self, code, path, token=None):
        with self.assertRaises(HTTPError) as error:
            self.request(path, token=token)
        self.assertEqual(error.exception.code, code)
        return error.exception.headers

    def test_returns_playbacks_like_the_page(self):
        _, headers, body = self.request("/public/v1/now-playing", token=APP_KEY)

        self.assertEqual(headers["Content-Type"], "application/json")
        self.assertEqual(headers["Access-Control-Allow-Origin"], "*")
        self.assertEqual(json.loads(body), self.state.snapshot())
        self.assertEqual(json.loads(body)[0]["user"], "alice")

    def test_accepts_key_as_query_parameter(self):
        status, _, _ = self.request(f"/public/v1/now-playing?api_key={APP_KEY}")
        self.assertEqual(status, 200)

    def test_rejects_missing_wrong_and_plugin_credentials(self):
        for token in (None, "wrong-key-0123456789", "secret"):
            headers = self.assert_status(401, "/public/v1/now-playing", token=token)
            # Browsers only show the error to the calling app with this header.
            self.assertEqual(headers["Access-Control-Allow-Origin"], "*")

    def test_api_key_does_not_open_internal_endpoints(self):
        for path in ("/api/now-playing", "/manifest.json", "/index.html"):
            self.assert_status(401, path, token=APP_KEY)

    def test_cors_preflight(self):
        status, headers, _ = self.request("/public/v1/now-playing", method="OPTIONS")

        self.assertEqual(status, 204)
        self.assertEqual(headers["Access-Control-Allow-Origin"], "*")
        self.assertIn("Authorization", headers["Access-Control-Allow-Headers"])


class ParseApiKeysTest(unittest.TestCase):
    def test_splits_and_trims(self):
        self.assertEqual(
            parse_api_keys(" a-key-0123456789abc , b-key-0123456789abc,", "token"),
            ["a-key-0123456789abc", "b-key-0123456789abc"],
        )

    def test_unset_disables(self):
        self.assertEqual(parse_api_keys(None, "token"), [])
        self.assertEqual(parse_api_keys("  ", "token"), [])

    def test_rejects_short_keys_and_the_plugin_token(self):
        with self.assertRaises(ValueError):
            parse_api_keys("short", "token")
        with self.assertRaises(ValueError):
            parse_api_keys("plugin-token-0123456789", "plugin-token-0123456789")


if __name__ == "__main__":
    unittest.main()
