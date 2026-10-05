# Architecture: layers, modules, dependency rules

## 1. Process model

```text
/usr/bin/at-webserver            daemon: sole AT owner, StateCache, EventBus,
                                 AT scheduler, module services, WS + TCP RPC
  ├── argv0 mt5700m-at           CLI client: forwards to the daemon, renders text
  └── argv0 atprobe              operator-only port probe (exclusive lock,
                                 refuses to run while the daemon holds the tty)

LuCI   ucode plugin ──nc──▶ daemon TCP RPC (newline JSON)  ── rpcd → views
WebUI  WebSocket ─────────▶ daemon WS RPC                  ── React pages
```

There is no second backend. The historical `mt5700m-manager` shell script and
the `mt5700m-traffic` accounting daemon remain as *system helpers* invoked by
init/rpcd glue; their AT-visible behaviour is routed through the daemon (see
[data-flow.md](data-flow.md) §6). `mt5700m-traffic` is the single writer of the
traffic history file and is read by both the backend and LuCI through one JSON
interface.

## 2. Backend layers (bottom-up)

| Layer | Path | Responsibility | May depend on |
| ----- | ---- | -------------- | ------------- |
| core | `src/core/` | error model, JSON, runtime, task primitives, sha1, `AtChannel` trait | std only |
| serial | `src/serial/` | exclusive tty open, USB presence monitor, probe | core |
| scheduler | `src/scheduler/` | AT arbiter (queues, priorities, retries, dedup, duty cycle), task manager, read gate, command plan | core, serial, state |
| state | `src/state/` | `StateCache` (topic TTLs), `EventBus` (coalescing, history), `RefreshCtx` | core |
| modules | `src/modules/<name>/` | one business domain each: commands, parser, state, service, api | core, state, scheduler (channel only) |
| transport | `src/transport/` | WS server, TCP RPC/control socket, URC dispatcher, AT client (CLI forwarding), channel | core, scheduler, modules (parsers only) |
| api | `src/api/` | registry + envelopes + CLI adapter | all of the above |
| daemon | `src/daemon.rs` | composition root: wires serial → arbiter → tasks → modules → transports | all |

Dependencies point **downward**. A module never reaches up into `daemon`,
`transport` or `api`; the daemon wires modules in, it is not called by them.

## 3. Module contract

Every module is a directory with the same five files plus `mod.rs`:

```text
src/modules/<name>/
├── mod.rs        what the module owns, in one paragraph
├── commands.rs   AT command construction (consts + builders)
├── parser.rs     AT response -> typed state (unit-tested here)
├── state.rs      domain model + to_json()/from_json() (wire shape lives here)
├── service.rs    refresh policy: AT via RefreshCtx, publish via store()/stale(),
│                 spawn() registering periodic jobs
└── api.rs        routes() -> Vec<Route> + text renderers for the CLI
```

Rules enforced by review and by the static checks in
`scripts/`-adjacent tooling described in [migration.md](migration.md):

* `service.rs` sees only `RefreshCtx` (AT + cache + bus) — never `AtArbiter`,
  never a tty, never another module's state struct internals.
* Cross-module data flows through cached topics (`ctx.cache.get(TOPIC_X)`) or a
  module's `cached*` service function; cross-module AT never happens twice.
* All wire JSON is produced by `state.rs::to_json`; the CLI's `key=value` text
  is a rendering of the same struct (`to_text`), so a parsing fix cannot land
  on one surface only.

Modules present after this refactor:

| Module | Topics | Routes | Owns |
| ------ | ------ | ------ | ---- |
| `signal` | `signal` | `signal.get`, `signal.cached` | `^HCSQ?` math |
| `network` | `network`, `registration` | `network.get`, `network.cached`, `registration.get`, `network.pdp`, `network.dhcp`, `network.registration_urc`, `network.lock_get`, `network.lock_apply`, `network.c5goption`, `network.c5goption_set`, `network.radio`, `network.radio_set`, `network.syscfg`, `network.syscfg_set` | `+COPS?`, `^SYSINFOEX`, `+C5GREG/+CEREG/+CREG`, `^LTEFREQLOCK?`/`^NRFREQLOCK?` (inc. the grouped-CSV write and the radio-cycle apply), `^C5GOPTION`, `^DHCP?`/`^DHCPV6?`/`^IPV6CAP?` |
| `cell` | `cell` | `cell.get`, `cell.cached`, `cell.neighbors` | `^HFREQINFO?`, `^MONSC` (per-RAT offsets), `^MONNC` (neighbours + ARFCN→band table) |
| `beam` | `beam` | `beam.ssb` | `^NRSSBID?` (SSB ids per serving/neighbour cell) |
| `ca` | `ca` | `ca.get`, `ca.cached` | `^HFREQINFO?` groups, `^CASCELLINFO?`, `^MONSSC` (carrier aggregation) |
| `qos` | `qos` | `qos.get`, `qos.cached` | `+CGACT?` active context, `^DSAMBR` AMBR/APN, `+CGEQOSRDP` QCI |
| `sim` | `sim` | `sim.get`, `sim.cached`, `sim.number`, `sim.slot`, `sim.slot_set`, `sim.hotplug_set`, `sim.pin_status`, `sim.pin_apply` | `+CPIN?` (inc. the CME-error branch), `^ICCID?`, `+CIMI`, `+CNUM`, `^SIMSQ?`, `^SCICHG`/`^TDSIMHP`/`^HVSST` (slot switch), `+CLCK`/`+CPWD` (PIN enable/change) |
| `modem` | `modem`, `txpower`, `nr_txpower`, `endc` | `modem.get`, `modem.cached`, `modem.txpower`, `modem.endc`, `modem.nr_txpower`, `modem.mcs`, `modem.reset`, `modem.imei_set`, `modem.nr_capability`, `modem.nr_capability_set` | `ATI`, `+CGSN`, `^TXPOWER?`, `^NTXPOWER?`, `^LENDC?`, `^MCS`, `AT^RESET`, `^PHYNUM=IMEI`, `^NRRCCAPQRY`/`^NRRCCAPCFG` (CA / VoNR / DSS) |
| `traffic` | `traffic`, `netrate` | `traffic.get`, `traffic.cached`, `traffic.netrate`, `traffic.clear` | `^PDCPDATAINFO?`, `^DSFLOWCLR`, interface counters, accounting report |
| `system` | `temperature` | `system.temperature`, `system.temperature.cached`, `system.device_control`, `system.nic_rate_set`, `system.power_control_set`, `system.factory_reset`, `system.service_mode`, `system.thermal`, `system.thermal_set` | `^CHIPTEMP?`, `^TDPCIELANCFG`, `^TDPMCFG`, `AT&F`, `^THERMAUTOFUN`/`^THERMLDLOGSW`/`^THERMLDAUTOPARA`/`^THERMLDAUTOSTATUS` |
| `sms` | `sms` | (in progress) | SMS-SUBMIT PDU codec, send transaction |

## 4. Frontends

| Frontend | Entry | Data access | Owns |
| -------- | ----- | ----------- | ---- |
| LuCI | `luci-app-mt5700m/htdocs/.../view/mt5700m/*.js` | ucode plugin (`mt5700m.at/cached/events`) → daemon RPC | rendering, tabs, dialogs, form state |
| WebUI | `semi-tcpweb/src/pages/*` | `services/at.ts` WS transport + `services/stateCache.ts` topic store | rendering, tabs, dialogs, form state |

Neither frontend imports the other; they share no files, no storage keys and no
state. Each can be built, deployed, updated or broken without touching the
other. Their only common dependency is the API contract in
[api-contract.md](api-contract.md).

## 5. Concurrency model

* One OS thread per transport listener (WS, control socket, TCP RPC), one
  reader thread for the tty, one executor thread in the arbiter.
* All AT requests are *messages* (`AtRequestSpec`) on the arbiter's priority
  queues with a budget (queue timeout + command timeout + slack). Nothing
  blocks while holding the tty except the command currently being executed.
* Periodic work is a `TaskManager` job: cancellable (task flag), timeout-able
  (per-job budget), retry-able (policy per command class), observable (task
  registry + `task.*` events), and non-leaking (jobs are reaped on completion).
* A failing job can only fail its own tick: `RefreshCtx` turns timeouts into
  "no answer" and falls back to the cached value, so one bad modem response
  cannot crash the daemon, a route or a page.
