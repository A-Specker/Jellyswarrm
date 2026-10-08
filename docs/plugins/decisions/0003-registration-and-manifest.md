# 0003 Registration and manifest

**Status:** accepted

## Context

Jellyswarrm needs to know which plugins exist, where to reach them, and what they
offer (for example a UI page). The admin should stay in control of what runs.

## Decision

* Plugins are registered **statically** in `jellyswarrm.toml` as a `[[plugins]]`
  list with `name`, `url`, `token` and `enabled`. This follows the existing
  `preconfigured_servers` pattern (`PreconfiguredServer` in `src/config.rs`).
* Changes are picked up on startup and by the existing config reload in the
  admin settings (`reload_config` in `src/ui/admin/settings.rs`).
* Each plugin describes itself in a **manifest**, `GET {url}/manifest.json`, with
  `name`, `version`, `api_version` and an optional `ui` block.
* A plugin whose `api_version` is not supported is disabled and a warning is
  logged. Jellyswarrm keeps running.

## Alternatives considered

* **Self-registration**: plugins announce themselves to Jellyswarrm. Convenient,
  but any service in the network could register, so it would need its own
  trust mechanism.
* **Managing plugins in the admin UI and the database.** Nicer for users, but
  needs migrations, forms and validation. It can be added later on top of the
  same manager.
* **No manifest, everything in the config.** Duplicates information the plugin
  knows best (version, UI title), and can't detect API mismatches.

## Consequences

* Adding a plugin means editing the config and reloading. No database migration
  is needed, which also avoids migration conflicts with upstream.
* An unreachable plugin is shown with status `unreachable` and retried on the
  next reload. It never stops Jellyswarrm from starting.
