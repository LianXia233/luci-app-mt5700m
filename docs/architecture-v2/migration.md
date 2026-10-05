# Migration log and status

## 1. What this refactor changed

**Stage A — relocation (commit `375a36c`).** The flat 20-file `src/` tree became
the layered tree (`core/`, `serial/`, `scheduler/`, `state/`, `transport/`,
`api/`, `modules/`) with `mod.rs` documentation and legacy names kept as local
aliases, so the move itself changed no behaviour.

**Stage B — unified API + first module (commit `448f0c8`).**

* `api/registry.rs`, `api/rpc.rs`: one route table, three thin envelopes
  (WebSocket, control socket, JSON-RPC). `daemon.rs` gained the `api` verb, the
  `api` RPC method and `api.*` dispatch, all reaching the registry.
* `core/channel.rs` (`AtChannel`), `scheduler/channel.rs` (`TaskChannel`,
  `DirectChannel`, `run_in_task`), `transport/channel.rs` (`DaemonChannel`),
  `state/refresh.rs` (`RefreshCtx`): the interfaces modules actually see.
* `modules/signal/*`: first module; HCSQ math exists once, five parser tests.
* Single AT owner enforced: the CLI's control-socket → serial → network cascade
  and its direct serial/raw-TCP SMS fallback are deleted; client port detection
  is descriptor-only; a missing daemon is a reported error.
* Build fix: `modules/sms/mod.rs` was missing after `sms.rs` moved to
  `modules/sms/pdu.rs`, so `pub mod sms;` did not resolve. The crate did not
  compile at Stage A; it does now.

**Stage C — every topic has a module (commit `dbb7eb0`).**

* `state/collectors.rs` (1020 lines) deleted. `cell`, `sim`, `modem`, `system`
  and `traffic` modules now own their collectors, parsers, backoff keys and
  cadences; `network` owns `+COPS`/`^SYSINFOEX`/`+CxxREG`.
* Deduplication: the PDCP field table moved out of `transport/urc.rs` into
  `modules/traffic/parser.rs` (URC stream and query path share it); the CLI's
  copies of the COPS/SYSINFOEX decoders and of the HCSQ math are gone (it
  aliases the module parsers and renders `SignalState`); the cell collector no
  longer issues a second `AT+COPS?` — it reads the network module's state.
* `scripts/ci-annotate-cargo.py` + CI step: cargo diagnostics are re-emitted as
  GitHub annotations, because this repository's CI logs are served from a blob
  URL that some environments cannot fetch.
* `scripts/rs-static-check.py`: the cargo-free structural checker described
  above, now part of the repository and run by CI before `cargo test` (it also
  checks enum variants, so a renamed/removed variant cannot slip through).
* `daemon.rs`: the dead one-shot SMS writer (`AtClient::send_sms` and
  `send_blocking`, which bypassed the arbiter) is deleted. Sending an SMS now
  exists exactly once: `AtTransport::send_sms` → `send_pdu` → `+CMGS` on the
  arbiter, reached by the control socket's `sms` verb, the WebSocket route and
  the CLI alike.

## 2. Verification performed

| Check | Result |
| ----- | ------ |
| Rust syntax (lezer parser, all 79 files) | clean |
| Static checks (`scripts/rs-static-check.py`: 338 `crate::` paths, 116 modules, local calls, trait impls, struct literals) | clean (self-tested: it flags a missing trait method and an unknown struct field) |
| Unit tests added with the modules | 31 tests across `signal`, `network`, `cell`, `sim`, `modem`, `traffic`, `system` |
| `cargo test --locked` (CI) | see the run on the head commit |
| Shell/JS/JSON/PO checks (CI `static-checks`) | see the run on the head commit |
| UI files touched by this refactor | none (by design) |

## 3. What remains (in order)

1. **Remaining modules**: `qos` (`+CGEQOSRDP`), `beam` (beam/scan commands),
   `diagnostics` (`cellscan`, port scan), `sms` service (list/send/receive
   behind routes). Each follows [module-guide.md](module-guide.md); the CLI
   verbs are the source of truth until they move. (`ca` landed with this
   commit: `modules/ca/{commands,state,parser,service,api}.rs` owns carrier
   aggregation, the CLI's `append_hfreqinfo_line`/`CaTotals` are gone, and the
   frozen `carrier_*`/`ca_*` text comes from `CaState::to_text()`.)
2. **CLI adapter thinning**: `api/cli.rs` still holds the per-verb text
   formatters for those capabilities; they move into the modules' `api.rs`
   `render_text` as the modules land.
3. **Frontend AT removal**:
   * WebUI — display already flows through the daemon's topic cache
     (`services/stateCache.ts` + `useSharedStateTopic`); the remaining AT usage
     is action/refresh paths in `services/at.ts` + `modem/*.ts`, which convert
     to the corresponding routes as they land. `pages/at/Terminal.tsx` stays a
     deliberate raw-AT console (user-facing diagnostic tool, admin-only).
   * LuCI — `parser.js` decodes CLI text for the pages that still call
     `mt5700m-at status/advanced …`; those pages move to the routes above and
     the duplicated decoders are deleted. The dashboard already reads the
     daemon cache via `api.cachedSnapshot()`.
4. **Fold `mt5700m-traffic` into `modules/traffic`** (last non-Rust business
   process) and the dialing glue of `mt5700m-manager` into `modules/network`
   actions.
5. **Split the two god files** (`daemon.rs` ~1.9k lines, `api/cli.rs` ~2.4k):
   `daemon.rs` → `transport/{ws_server,rpc_server,control_server}` +
   `api/rpc.rs`; `api/cli.rs` → one `render_text` per module + a small verb
   table. Both are now pure wiring/adapters, so the split is mechanical.
