# Module guide: adding a feature

A feature is a module. It is never a new parser in a frontend, a new branch in
`daemon.rs`, or a second copy of an existing decoder.

## 1. Layout

```text
src/modules/<name>/
├── mod.rs        one paragraph: what this module owns
├── commands.rs   AT command constants/builders
├── state.rs      typed domain model + to_json()/from_json() (+ tests)
├── parser.rs     AT text -> typed state (+ tests, one per response variant)
├── service.rs    refresh policy + spawn() of periodic jobs
└── api.rs        routes() + text renderers for the CLI
```

## 2. Steps

1. **Commands** — put every AT literal the domain needs in `commands.rs`.
   Nothing else in the module may contain an AT string.
2. **Parser** — decode the response into a struct in `state.rs`. Tests go in
   `parser.rs` (one per variant: happy path, vendor variant, error/absent).
3. **State** — `to_json()` is the wire contract; keep the field names existing
   consumers already read. If the CLI must print text, add `to_text()` here so
   JSON and text can never disagree.
4. **Service** — read with

   ```rust
   ctx.read(cmd, at_timeout, queued_timeout, Priority::Low)?  // background
   ctx.slow(key, cmd, at_timeout, queued_timeout, backoff)     // backoff-gated
   ctx.query(cmd)?                                            // interactive read
   ctx.action(cmd)?                                           // write
   ```

   then publish with `ctx.store(TOPIC, EVENT, &state.to_json())` or fall back
   with `ctx.stale(TOPIC, EVENT)`. Never touch `AtArbiter`, a tty or a socket.
5. **Spawn** — call `run_in_task` from `spawn(tasks)`; register the job in
   `modules::spawn_all`. Pick the period/TTL pair that matches the topic's
   freshness promise.
6. **API** — add `Route`s and register them in `api/registry.rs::routes()`.
7. **Wire** — `mod.rs` (docs), `modules/mod.rs` (module + spawn), registry
   (routes). That is the whole registration surface.

## 3. Checklist before committing

* No AT literal outside `commands.rs`; no `crate::serial`, `crate::scheduler::arbiter`
  or `std::net`/`std::fs` tty access in a module.
* Every new response shape has a unit test with a real modem reply captured
  from the field (truncated/odd variants included).
* `python3 scripts/rs-static-check.py mt5700webui-openwrt-server/at-webserver/src`
  (module tree, `crate::` paths, call arity, local calls, trait impls, struct
  fields, enum variants, stacked derives, unimported module-qualified calls and
  overlapping closures on one local) and `cargo test` are clean; both run in
  CI, and the checker's checks are themselves mutation-tested (see the "Errors &
  dead ends" list in `migration.md`).
* Frontend diff contains **no** AT strings and no new business rule.
