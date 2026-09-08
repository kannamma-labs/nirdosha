# nirdosha-plugin-native-authed-http

The RFC 0011 reference for a real `call`-shape plugin
(rfcs/0011-uniform-service-provider-model.md §2/§4/§8): a provider for
the `authedhttp://` scheme, registered via `call_via`'s runtime plugin
fallback, that makes a genuine HTTP/1.1 request with an
`Authorization: Bearer <token>` header — the concrete gap the RFC's own
Motivation names (`http_get`/`http_post`/`call_via`'s two core-served
schemes have no way to set a custom header from `.nir` at all).

## Try it

```sh
AUTHEDHTTP_BEARER_TOKEN=some-secret cargo build --release -p nirdosha-plugin-native-authed-http
# produces target/release/libnirdosha_plugin_native_authed_http.a
```

`crates/compiler/tests/native_plugin_examples.rs` builds this exact
crate, spins up a real local `tiny_http` server on an ephemeral port,
links this plugin into a real `nirdosha build` output, compiles a `.nir`
program that calls `call_via("authedhttp://127.0.0.1:<port>", ...)`, and
asserts the server actually observed the expected `Authorization`
header — `cargo test -p nirdosha --test native_plugin_examples`.

## The four functions

```toml
[package.metadata.nirdosha]
kind = "native-compiled"
builtins = [
    { name = "authedhttp_provider_authedhttp_connect", params = ["str"], ret = "i64" },
    { name = "authedhttp_provider_authedhttp_request", params = ["i64", "str", "str"], ret = "str" },
    { name = "authedhttp_provider_authedhttp_is_valid", params = ["i64"], ret = "i64" },
    { name = "authedhttp_provider_authedhttp_close", params = ["i64"], ret = "i64" },
]
```

`_connect` reads `AUTHEDHTTP_BEARER_TOKEN` from the process environment
once, at connect time, and stashes it (with the target `host:port`)
against a fresh handle — the credential-at-connect-time shape RFC 0011
§2 specifies for `conn`/`call`-shape providers alike. `_request` makes
one real `TcpStream`-backed HTTP/1.1 request per call (no keep-alive on
this side — that's the kernel's own `PoolRegistry<
PluginManagedConnection>` job, Phase 5), with the stashed token set as
`Authorization: Bearer <token>`, and returns the response body as plain
`str` (no separate status-code channel — `call_via`'s own disclosed
cost, see `nir_call_via`'s doc comment in `runtime-kernels`).

## What's honestly not covered

- **No `Result`/status-code crossing** — same `str`-only ABI limit every
  other native plugin in this repo has (`plugin-example-native-kv`'s own
  README, "What's honestly not covered"). A transport failure comes back
  as an `"ERR: ..."`-prefixed body string, not a distinguishable error
  channel.
- **No connection reuse inside the plugin itself** — deliberate; see
  `src/lib.rs`'s own module doc for why pooling belongs one layer up, in
  the kernel.

## Full design

rfcs/0011-uniform-service-provider-model.md.
