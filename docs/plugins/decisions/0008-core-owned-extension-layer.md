# 0008 Core-owned extension layer

**Status:** proposed

## Context

v1 kept all plugin code in `src/plugins/` and limited core changes to a few
touch points ([0007](0007-fork-hygiene.md)), so the fork stays easy to rebase.
That worked for a read-only viewer, but it blocks the next features:

* Plugins can't act (no commands) or change behaviour (no request hooks).
* Events come from one place, so sessions that end without a stop report are
  invisible to plugins.
* One permanent shared secret per plugin, with no limits per permission.
* Plugin pages run on the admin's origin and must be fully trusted.

Fixing these needs changes across the core: event sources in several modules,
hooks in the translation pipeline, database tables, and a new admin page. That
is not maintainable as a fork that has to follow upstream `main`.

## Decision

* Build the plugin system as a **core-owned extension layer** and propose it to
  upstream, as described in the [v2 proposal](../proposal-v2.md).
* The core declares typed extension points: event bus, Query API, Command API,
  hook chain, providers, UI slots, scheduled tasks. **Each does nothing by
  default**; without plugins, behaviour is unchanged.
* A **plugin host** manages plugins: lifecycle, state in the database, pairing,
  short-lived tokens limited to approved permissions, health checks and circuit
  breakers.
* Plugins run **out-of-process** by default. An in-process WASM runtime is added
  only for request hooks that need low latency.
* Plugin pages run on a **separate origin** in a sandboxed iframe.
* `/plugin-api/v1` stays available through a compatibility layer.

If accepted, this decision supersedes 0007 and replaces parts of
[0002](0002-pluginmanager-as-bridge.md) (the manager as the only bridge),
[0004](0004-auth-tokens.md) (shared tokens) and
[0006](0006-ui-via-reverse-proxy.md) (same-origin pages).

## Alternatives considered

* **Keep extending v1 in the fork.** Each new feature adds touch points, and
  rebases get harder. Isolated UI and request hooks are hard to add without
  deeper changes.
* **In-process plugins only (dynamic libraries).** Fast, but Rust has no stable
  ABI, and a crashing plugin takes down the proxy.
* **WASM only.** Strong isolation and low latency, but harder for plugin authors
  and a poor fit for long-running work like event consumers or UIs.
* **Reuse Jellyfin's server plugin model.** It is .NET and in-process, and
  Jellyswarrm only acts like a Jellyfin server towards clients.

## Consequences

* Larger core changes: `application/`, `translation/`, `persistence/`,
  `startup/`, `ui/`. They only pay off if upstream accepts them.
* Plugin authors get a stable, versioned contract with generated client
  libraries, a mock host and conformance tests.
* A misbehaving plugin can't delay or break playback, and can only do what an
  admin approved.
* More to build and maintain in the core: host, tokens, audit log, migrations.
  The roadmap in the proposal splits it into six steps that each work on their
  own.
* If upstream declines, we fall back to v1 and add only the smallest pieces
  (for example `session.ended`) to the fork.
