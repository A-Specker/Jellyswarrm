# 0001 External plugins

**Status:** accepted

## Context

We want to add features, starting with a "who is watching" viewer, without
carrying them as code changes in a fork. Jellyswarrm is a single Rust binary
without any plugin mechanism.

## Decision

Plugins are **external services**: separate processes or containers that talk to
Jellyswarrm over HTTP. They can be written in any language.

## Alternatives considered

* **Compiled-in Rust crates** behind Cargo features. Fastest and most powerful,
  but every plugin depends on internal types that change upstream, and every
  plugin change needs a Jellyswarrm rebuild. This moves the fork problem into the
  plugins instead of solving it.
* **WASM modules** loaded at runtime (wasmtime, Extism). Sandboxed and
  language-independent, but by far the most work: host functions, memory limits,
  and serving a UI from inside WASM is awkward.
* **Dynamic libraries.** Rust has no stable ABI, so plugins would break with
  compiler and dependency updates.

## Consequences

* Plugins are independent of Jellyswarrm's internals and release cycle.
* Plugins cannot change proxy behaviour directly; they only see what the plugin
  API exposes.
* Each plugin is one more service to run. For a home setup this is one more
  container in the same compose file.
* Calls go over HTTP, which is slower than in-process calls. This is fine for
  read-only data and events, the v1 scope.
