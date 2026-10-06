# Data flow and the single AT owner

## 1. The pipeline

```text
AT response ──▶ modules/<x>/parser.rs ──▶ modules/<x>/state.rs (typed)
                                             │
                              to_json()      │      to_text()   (CLI only)
                                 ▼           ▼
                        StateCache topic ──▶ EventBus ──▶ WS push / events RPC
                                 │
                                 ▼
                     api/registry.rs route ──▶ WS envelope / control socket /
                                               JSON-RPC / CLI stdout
```

One direction, one writer per topic, one decoder per response. A frontend
cannot reach the AT layer: it can only call a route or read a topic.

## 2. Who writes which topic

| Topic | Writer (module service) | Cadence | Failure behaviour |
| ----- | ----------------------- | ------- | ----------------- |
| `signal` | `signal::service::refresh` | 15 s | stale fallback, 120 s backoff |
| `network` | `network::service::refresh` | 30 s | 60 s / 300 s backoff per command |
| `registration` | `network::service::refresh_registration` | 20 s | topic always written |
| `cell` | `cell::service::refresh` | 120 s | per-command backoff, TAC/CI from `registration` |
| `ca` | `ca::service::refresh` | **on demand** (`ca.get`) | previous picture kept, 60/120/300 s per-command backoff |
| `qos` | `qos::service::refresh` | **on demand** (`qos.get`) | partial values kept, 60/120 s per-command backoff |
| `sim` | `sim::service::refresh` | 60 s | per-command backoff |
| `modem` | `modem::service::refresh_info` | 60 s | 60 s backoff, explicit 60 s TTL |
| `txpower` / `endc` | `modem::service::refresh_*` | 300 s | 600 s backoff |
| `nr_txpower` | `modem::service::refresh_nr_txpower` | 180 s | 600 s backoff |
| `temperature` | `system::service::refresh` | 60 s | stale fallback |
| `traffic` | `traffic::service::refresh` | 30 s | empty object |
| `netrate` | `traffic::service::refresh_netrate` | 5 s | `available:false` + reason |
| `scan` | `cell::scan::start` (exclusive task) | **on demand** (`cell.scan_start`) | one `{state, cells, count}` push per scan; `cell.scan_state`/`cell.scan_abort` are task introspection |
| `fota` | `system::fota::start` (long-running task) | **on demand** (`system.fota_start`) | one `fota.progress` push per state change; `system.fota` answers from the snapshot, `system.fota_abort` cancels the task |
| `schedule` | `network::schedule::write` (UCI) | on save | not a modem topic: `scheduler::plan` re-reads UCI every 15 s, `network.schedule_get` reads it on page load |
| `sms`, `task`, `usb`, `beam`, `raw:*` | daemon/URC/dispatch paths | event-driven | topic-specific. The URC path is the decoder for everything the modem volunteers: `+CUSD` → `sms.ussd` (`modules::sms::ussd`), `^REJINFO` → `network.reject` (`modules::network::reject`), `^DSAMBR` → `qos.ambr` (`modules::qos`), `+CPIN`/`^SIMSQ`/`^SIMST` → `sim.changed`, and the raw line still goes out as `raw_data` for the diagnostic views |

TTLs live in `state/cache.rs`; publishing goes through
`RefreshCtx::store`/`stale`, which is the only place that sets a topic *and*
emits its event (so a topic cannot be updated silently).

## 3. Single AT owner, in sequence

```text
module service / route handler
        │  ctx.read(cmd, at_timeout, queued_timeout, prio)  |  ctx.slow(...)
        ▼
core::channel::AtChannel            (trait the modules see)
        │
        ├── scheduler::channel::TaskChannel    (inside a periodic job)
        └── scheduler::channel::DirectChannel  (inside a request handler)
        ▼
scheduler::arbiter::AtArbiter  ── priority queue + read gate + dedup + retry
        ▼
scheduler::plan + transport::urc (URC demux) ──▶ daemon AtClient
        ▼
serial::manager  ── exclusive tty (TIOCEXCL, O_NOCTTY, O_NONBLOCK)
        ▼
modem
```

Out-of-process callers hit the same arbiter:

```text
mt5700m-at  ──▶ transport::client::at_cmd ──▶ control socket "send" verb ──┐
LuCI ucode  ──▶ daemon TCP RPC              ──▶ transport::urc path   ─────┤
WebUI       ──▶ WS RPC / api.<module>.<verb> ───────────────────────────▶ arbiter
```

The CLI has **no** direct serial or raw-TCP fallback: if the daemon is not
running, `at_cmd` returns a `DaemonFailed` error naming the init script. This is
deliberate — a second writer on the tty is not a degraded mode, it is a broken
modem session (and a deadlock against the daemon's own lock). The same applies
to SMS: `mt5700m-at sms-send` calls the daemon's `sms.send` route, which encodes
the PDU and runs the whole `+CMGS` transaction under the arbiter — the CLI owns
neither a PDU codec nor a send sequence.

Port detection from a client is descriptor-based only (`detect_pcui_port`); only
the daemon may run an AT probe (`auto_detect_serial`), and only before it takes
ownership. `atprobe` exists for field diagnostics and opens the port
exclusively, so it refuses to run while the daemon is alive.

## 4. Task lifecycle

```text
TaskManager::add_periodic(name, period, priority, timeout, f)
   │
   ├── spawns at daemon start (modules::spawn_all → <module>::service::spawn)
   ├── each tick: run_in_task(ctx, f) builds TaskChannel + RefreshCtx
   ├── deadline: queue timeout + AT timeout; on overrun the tick is abandoned
   ├── cancel: task flag checked by the arbiter before/while writing
   ├── errors: mapped to BackendError (never a panic); stale fallback keeps UI
   └── observability: task registry + `task.*` events → both frontends
```

## 5. Error model

`core::error::BackendError` is the single error type crossing every boundary
(serial, arbiter, modules, routes, CLI). Each variant carries a stable `code()`
(`AT_TIMEOUT`, `BUSY`, `MODEM_UNAVAILABLE`, `INVALID_PARAMETER`, …) and a
`retryable()` flag that the WS envelope exposes:

```json
{"success": false, "error": "…", "code": "AT_TIMEOUT", "retryable": true}
```

Frontends switch on `code`, never on message text.

## 6. The two remaining system helpers

| Helper | Role | Plan |
| ------ | ---- | ---- |
| `mt5700m-traffic` (init-daemon + rpcd) | counts interface bytes into `/etc/mt5700m/traffic-history`, answers `json`/`summary` for LuCI and the backend | read through one JSON interface today; folding the counting loop into `modules/traffic` is the next step (removes the last non-Rust business process) |
| `mt5700m-manager` + rpcd shim | dialing glue (`AT^NDISDUP`, `AT+CFUN`, `^SETMODE`) invoked by hotplug/init and LuCI's dial buttons | its AT calls already go through `mt5700m-at` → daemon; folding these into `modules/network` actions (dial/redial/airplane) is the following step |
