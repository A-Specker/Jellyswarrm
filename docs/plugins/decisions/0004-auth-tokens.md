# 0004 Authentication with tokens

**Status:** accepted

## Context

The plugin API exposes who is watching what, which is personal data. Plugin UI
pages run inside the admin area. Both directions need to be authenticated.

## Decision

* Each plugin has **one random token** in its `[[plugins]]` entry.
* **Plugin → Jellyswarrm:** plugin API requests must send
  `Authorization: Bearer <token>`. The token identifies the plugin. Tokens are
  compared in constant time. Missing or wrong tokens get `401`.
* **Jellyswarrm → plugin:** manifest and UI requests carry the same header, so a
  plugin can reject traffic that doesn't come from Jellyswarrm.
* The API is **read-only** in v1. A token grants read access to the plugin API
  and nothing else; it is not a Jellyfin API key and doesn't work on any other
  route.

## Alternatives considered

* **Reuse Jellyfin API keys** (Jellyswarrm already has `virtual_user_api_keys`).
  Those keys belong to users and grant access to the Jellyfin API, which is much
  more than a plugin should get.
* **No authentication on a private network.** Too easy to expose by accident,
  for example through a reverse proxy.
* **Separate tokens per direction, or scopes per endpoint.** More secure in
  theory, but more configuration for no real gain while the API is read-only.
  Scopes can be added when write access arrives.

## Consequences

* Leaking a token exposes read access to sessions, users and servers, so tokens
  must be treated like passwords. The admin UI never displays them.
* Rotating a token means changing it in the config and in the plugin, then
  reloading.
