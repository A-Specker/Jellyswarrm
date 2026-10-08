# 0006 Plugin UI via reverse proxy

**Status:** accepted

## Context

Plugins like the viewer need a page in the admin UI. The admin UI is rendered
with askama templates and htmx under `/{ui_route}`, protected by a login and the
`require_admin` middleware in `src/ui/mod.rs`.

## Decision

* Jellyswarrm **proxies** `/{ui_route}/plugins/{name}/{*path}` to
  `{url}/{path}` of the plugin.
* The route sits behind the existing **`require_admin`** middleware, so only
  logged-in admins can open plugin pages.
* Before forwarding, Jellyswarrm **removes the session cookie** and adds the
  plugin's token (see [0004](0004-auth-tokens.md)).
* For every plugin whose manifest has a `ui` block, the admin page gets a **tab**
  with `ui.title` and `ui.icon`, rendered in `templates/admin/index.html` through
  `AdminIndexTemplate` (`src/ui/root.rs`).
* Plugin pages must use **relative URLs**.

## Alternatives considered

* **Link to the plugin's own URL.** No integration and no shared login: each
  plugin would need its own authentication and be exposed separately.
* **iframe of the plugin's URL.** Still needs the plugin to be reachable from the
  browser and to handle authentication itself.
* **Plugins return HTML fragments for htmx.** Tighter visual integration, but
  ties plugins to Jellyswarrm's templates and CSS, which can change upstream.

## Consequences

* Plugins don't need their own login and don't have to be reachable from the
  internet; only Jellyswarrm needs to reach them.
* A plugin page runs on the same origin as the admin UI. This is why the session
  cookie is stripped, and why only configured plugins are proxied.
* Plugins that use absolute paths (`/assets/app.js`) break below the prefix.
  This is documented in the [Plugin API](../plugin-api-v1.md#ui-pages).
