# 0002 PluginManager as a bridge

**Status:** accepted

## Context

All of Jellyswarrm's runtime state lives in `AppState` (`src/main.rs`): client
sessions, users, servers, config. Plugins need some of this data. `AppState` and
its services change often upstream, and they also hold sensitive data: encrypted
server credentials, client tokens, and the admin password in the config.

## Decision

The **PluginManager** in `src/plugins/` is the only code that touches `AppState`
on behalf of plugins. It translates internal types into plain JSON objects (DTOs)
of a versioned API, `/plugin-api/v1` (see [Plugin API v1](../plugin-api-v1.md)).
Plugins never see internal types.

## Alternatives considered

* **Expose `AppState` directly** (only possible for compiled-in plugins, see
  [0001](0001-external-plugins.md)). Every upstream refactor could break plugins,
  and plugins could reach credentials.
* **Generic data access**, for example read access to the SQLite database or a
  query endpoint. Same coupling to internals, and most of the interesting state
  (who is watching) is only in memory, not in the database.

## Consequences

* When upstream changes internal types, only the mapping in `src/plugins/` needs
  fixing; plugins keep working.
* Every piece of data a plugin needs must be added to the API explicitly. This is
  deliberate: it keeps the exposed surface small and reviewed.
* Some small additions to core services are needed, for example
  `ClientSessionService::all()`, because today sessions can only be listed per
  user.
