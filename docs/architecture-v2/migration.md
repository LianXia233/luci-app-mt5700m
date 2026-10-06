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
  exists exactly once: the transport's PDU writer → `send_pdu` → `+CMGS` on the
  arbiter, reached by the WebSocket route and the CLI alike (the control
  socket's separate `sms` verb that Stage C still used was folded into that one
  path by Stage D).

**Stage D — the SMS module (commit `8383b7d` and its successor).**

* `modules/sms` gained `commands.rs`, `parser.rs`, `state.rs`, `service.rs` and
  `api.rs` next to `pdu.rs`, and now owns the whole flow: `+CMGF`/`+CMGL`/
  `+CMGD`/`+CPMS`/`+CSCA`/`^IMSSWITCH` construction, reply decoding, the
  `SmsMessage`/`SmsStorage`/`SmsSettings` domain model and the five-step IMS
  sequence. Nine routes (`sms.status|storage|list|send|delete|clear_all|
  storage_set|center_set|ims_set|analyze`) are registered in the same table as
  every other module's.
* `pdu.rs` became a two-way codec: the existing SMS-SUBMIT encoder plus an
  SMS-DELIVER decoder (GSM 7-bit alphabet, UCS-2, alphanumeric senders,
  semi-octet SCTS, UDH concatenation). Multipart parts are *merged* in the
  parser, so a list is a list of messages. The DCS predicate now tests bits 3..2
  (`0x08` is UCS-2, as the modem sends it) instead of the 0x04 bit alone.
* `core::channel::SmsPart` + a required `AtChannel::send_sms_pdu` replace the
  old number/text payload: the module encodes, the channel/transport performs
  the `AT+CMGS` transactions. `AtPayload::SMS { parts }`,
  `AtTransport::send_sms_pdu`, and the daemon's `send_pdu(length, hex, timeout)`
  carry the octets instead of the SMS PDU type, and the daemon names the failing
  part (`第 i/n 条发送失败：…`) — the copy the send page used to assemble.
* Deleted duplicates: the control socket's `sms` verb, the WebUI's
  `modem/smsEncode.ts` (node-pdu SMS-SUBMIT builder), `modem/sms.ts`'s
  `parseCMGL`/`processPDUMessage`/`mergeConcatenated` (node-pdu decode + merge),
  `@/types/node-pdu.d.ts` and the `node-pdu` dependency itself, `services/at.ts`'s
  `listAllSMS`/`getSMSStorage`, and the CLI's raw-AT IMS sequence / `+CMGD` /
  `+CPMS` / `+CSCA` verb bodies.
* The SMS pages (`pages/sms/Center.tsx`, `pages/sms/Settings.tsx`) now contain
  no AT command: the list, storage planes, centre number, IMS switch, clear-all
  and the compose hint all come from routes. The compose hint keeps its exact
  copy but asks `sms.analyze`, so the promised part count comes from the codec
  that sends.
* The sent-message cache stayed in the frontend **on purpose**: the UI calls it
  本地缓存 and ships export/import buttons for it, so it is browser-local data,
  not modem state. Everything derived from the modem is backend state.
* LuCI's own SMS view still reads the CLI's `sms-list`/`sms-info` text and
  decodes it in `parser.js`; unifying that view is the LuCI batch, which is also
  where `parser.js`'s SMS half gets deleted.

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

1. **Remaining modules**: `beam` (beam/scan commands),
   `diagnostics` (`cellscan`, port scan). Each follows
   [module-guide.md](module-guide.md); the CLI verbs are the source of truth
   until they move. `sms` landed: `modules/sms` now owns the PDU codec both ways
   (SMS-SUBMIT encode, SMS-DELIVER decode, GSM 7-bit / UCS-2, multipart split and
   merge), so `pages/sms/Center.tsx` and `pages/sms/Settings.tsx` contain no AT
   command at all, `modem/smsEncode.ts` and the `node-pdu` dependency are
   deleted, `modem/sms.ts` keeps only the browser-local sent-message cache and
   display formatters, and the CLI's `sms-send`/`sms-delete`/`sms-clear`/
   `sms-set`/`sms-ims` verbs run the same routes (its `sms-list`/`sms-info`
   verbs still print the raw text the LuCI view parses — that view and
   `parser.js`'s SMS half move in the LuCI batch). (`ca`, `qos` and the
   Info page's data-call fields landed already: carrier aggregation lives in
   `modules/ca` — the CLI's `append_hfreqinfo_line`/`CaTotals` are gone and the
   frozen `carrier_*`/`ca_*` text comes from `CaState::to_text()` —
   `modules/qos` owns `+CGACT?`/`^DSAMBR`/`+CGEQOSRDP` rendering both CLI verbs
   (`qci_text()`, `ambr_text()`) and serving `qos.get`, and `modules/network` /
   `modules/modem` / `modules/traffic` now own the Info page's remaining raw
   reads and writes (`network.dhcp` for `AT^DHCP?`/`AT^DHCPV6?`/`AT^IPV6CAP?`
   including the IPv4 hex decode, `modem.mcs` for `AT^MCS=1`/`=0`,
   `network.registration_urc` for the `AT+CGREG=2` side effect, `traffic.clear`
   for `AT^DSFLOWCLR`). `Info.tsx` therefore contains no AT command at all, and
   the dead AT verb set in `services/at.ts` (`sendSMS`, call control,
   `parsePDU`, registration queries) was deleted rather than left to rot.)
   The frequency lock went the same way: `modules/network` owns
   `^LTEFREQLOCK?`/`^NRFREQLOCK?` (row layout, hex PCI), the grouped-CSV write
   with its range tables, the radio-cycle apply sequence and the
   `^C5GOPTION` get/set triple, so the CLI's `lock`/`preview-lock` verbs, the
   day/night scheduler and the Settings page all build the same write through
   `commands::lte_lock_command`/`nr_lock_command` + `service::apply_lock`, and
   `modem/lock.ts` no longer builds an AT string. `modules/cell` owns the
   neighbour scan (`cell.neighbors`, `AT^MONNC`) including the ARFCN→band
   table the WebUI used to keep in `modem/parse.ts`. `modules/beam` owns the NR
   SSB report (`beam.ssb`, `AT^NRSSBID?`) with its fixed offsets, so the
   Settings page contains no AT command at all. `modules/sim` then took over the
   system page's whole SIM/PIN card: `sim.slot`/`sim.hotplug_set` for the slot
   and hot-plug switches, `sim.pin_status` for `+CPIN?` (including the CME-error
   branch, `^SIMSQ?` refinement and `+CLCK="SC",2`) and `sim.pin_apply` for the
   PIN verbs, so `modem/sim.ts` keeps only display text (code → label) and the
   CME-message translation, and `components/SimPinHandler.tsx` submits an
   operation instead of building an AT command. `modules/system` then took the
   system page's board cards (`system.device_control`, `system.nic_rate_set`,
   `system.power_control_set`, `system.factory_reset`, plus `system.service_mode`
   — the serial/network link kind recorded once at startup in `core::modem`, so
   the page no longer probes with `AT+CONNECT?`), `modules/modem` took
   `modem.reset` (`AT^RESET`) and `modem.imei_set` (`^PHYNUM=IMEI`, rule
   included), and `modules/network` took the airplane switch
   (`network.radio`/`network.radio_set`). The system page's identity card and NR
   transmit-power card now read `modem.get`/`modem.nr_txpower`. The only raw-AT
   surfaces left in the WebUI are the FOTA page and the deliberate
   `pages/at/Terminal.tsx` console. The cell-scan panel moved next:
   `modules/cell/scan.rs` owns the command builder (manual 5.35's constraints
   and the band bitmap), the `^CELLSCAN:` line parser and the exclusive task
   behind `cell.scan_start`/`cell.scan_state`/`cell.scan_abort`, pushing
   decoded cells on `cells[]`; `modem/cellscan.ts` (the WebUI's own parser and
   command builder) is deleted. LuCI's scan modal keeps reading
   `mt5700m-at cellscan`'s text — that verb now funnels into the same task via
   `scan::pseudo_command`, so the raw-AT surface is a compatibility shim, not
   a second implementation. *Open item, LuCI side:* its scan modal still reads
   that CLI text, which the asynchronous task cannot fill (the CLI is capped at
   ~25 s while a full-band scan takes minutes) — it should call
   `cell.scan_start` and render the `cellscan` push, with the same modal markup. The last three
   system-page cards followed: `modules/modem` owns the NR capability reads and
   writes (`modem.nr_capability`/`_set`, `^NRRCCAPQRY`/`^NRRCCAPCFG` for CA, VoNR
   and DSS with the reply-kind matching and the range rules), `modules/network`
   owns `network.syscfg`/`network.syscfg_set` (the seven-argument `^SYSCFGEX`
   write plus the CLI's `set-radio-policy` validation) and `modules/system` owns
   `system.thermal`/`system.thermal_set` (the four `^THERMLD*` reports and the
   master switch). `pages/system/Info.tsx` therefore contains **no AT command at
   all** — 35 raw call sites at the start of the session, 0 now.)
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
