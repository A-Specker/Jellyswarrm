# 0007 Fork hygiene

**Status:** accepted

## Context

The plugin system lives on a fork that has to follow upstream `main`. Every line
we change in shared files is a possible merge conflict on the next rebase.

## Decision

* All plugin code lives in **`crates/jellyswarrm-proxy/src/plugins/`**.
* Changes to existing files are limited to the **touch points** listed in the
  [roadmap](../ROADMAP.md#core-touch-points): module declaration, one
  `AppState` field, router merge, config field, config reload, one publish call,
  and the admin UI tabs.
* Touch points follow existing patterns, for example `health::router()` for
  mounting routes, so they look like upstream code.
* No database migrations for plugins (see [0003](0003-registration-and-manifest.md)).
* The branch is rebased onto `upstream/main` regularly, not merged, to keep the
  plugin changes as a small set of commits on top.

## Alternatives considered

* **Spread hooks wherever convenient.** Faster at first, but every rebase gets
  harder.
* **Upstream the plugin system.** The best outcome if the maintainer accepts it.
  Keeping the core changes small also makes such a proposal easier later.

## Consequences

* Some features will be slightly less convenient to build, because they must go
  through the API instead of reaching into core code.
* A rebase conflict is limited to a handful of known lines, and the plugins
  themselves are never affected.
