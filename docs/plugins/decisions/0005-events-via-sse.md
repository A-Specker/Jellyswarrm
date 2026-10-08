# 0005 Events via Server-Sent Events

**Status:** accepted

## Context

Plugins like the viewer need to react when playback starts, progresses or stops.
Every successful playback report already passes through one function,
`sessions::observe_playback()` in `src/sessions/mod.rs`. Whatever we add there
runs on the proxy's request path and must never slow it down.

## Decision

* The PluginManager owns a `tokio::sync::broadcast` channel. `observe_playback()`
  publishes a `PluginEvent` to it; publishing never blocks or fails the request.
* Plugins subscribe with `GET /plugin-api/v1/events`, a **Server-Sent Events**
  stream (supported by axum out of the box).
* Delivery is **best effort**: a subscriber that falls behind skips the missed
  events, and nothing is stored or retried.
* v1 events: `playback.started`, `playback.progress`, `playback.stopped`.

## Alternatives considered

* **Webhooks** (Jellyswarrm POSTs events to each plugin). Needs outbound queues,
  timeouts and retries in Jellyswarrm, and a slow plugin can build up memory.
  Can be added later as a separate delivery mode on top of the same channel.
* **WebSocket.** Bidirectional, which v1 doesn't need. More complex than SSE for
  the same result.
* **Polling only** (`GET /sessions` every few seconds). Simple and still useful
  as a fallback, but misses short playbacks and adds constant load.

## Consequences

* Plugins connect out to Jellyswarrm, so a plugin doesn't have to be reachable
  for events (it does for UI pages).
* Plugins must handle reconnects and gaps, typically by reloading
  `GET /plugin-api/v1/sessions` after reconnecting.
* `playback.progress` can be frequent with many clients. Plugins filter what they
  need; Jellyswarrm doesn't throttle in v1.
