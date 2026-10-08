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
* LuCI's own SMS view no longer reads the CLI: it asks `sms.list`/`sms.status`
  (the same routes the WebUI's two SMS pages call) and writes through
  `sms.send`/`sms.delete`/`sms.clear_all`/`sms.center_set`/`sms.storage_set`/
  `sms.ims_set`. `parser.js`'s SMS half (`parseMessages`, `parseInfo`,
  `decodePdu`, `decodeGsm7`, `decodeUcs2`, `swapDigits`) is deleted with it, so
  the PDU/GSM-7/UCS-2 codec exists once in the tree: `modules/sms::pdu`. Only
  `groupMessages` stays (a pure display grouping over the page's view model).

  Two consequences worth recording:

  * the decoded sender/recipient number is now the backend's domain value
    (digits, no `+` prefix). The old JS decoder prepended `+` when the PDU's TOA
    said "international": the digits are identical, the `+` is gone, and this is
    what the WebUI renders from the same field. `prove-sms-parity.js` pins that
    as the *only* value difference and counts the `+`s per shape, so the allowed
    difference cannot quietly grow.
  * the settings dialog's storage select and the slot badge both read the
    `+CPMS` **read** plane — what the old `parseInfo` regex picked up (its first
    match). The parity proof uses a fixture whose three planes differ, so an
    off-by-one plane cannot pass unnoticed.

## 2. Verification performed

| Check | Result |
| ----- | ------ |
| Rust syntax (lezer parser, all 79 files) | clean |
| Static checks (`scripts/rs-static-check.py`: 338 `crate::` paths, 116 modules, local calls, trait impls, struct literals) | clean (self-tested: it flags a missing trait method and an unknown struct field) |
| Unit tests added with the modules | 31 tests across `signal`, `network`, `cell`, `sim`, `modem`, `traffic`, `system` |
| `cargo test --locked` (CI) | see the run on the head commit |
| Shell/JS/JSON/PO checks (CI `static-checks`) | see the run on the head commit |
| UI files touched by this refactor | none (by design) |
| LuCI migration parity (`scripts/prove-neighbors-parity.js`, `prove-lock-parity.js`, `prove-network-parity.js`, `prove-cellscan-parity.js`, `prove-radio-parity.js`, `prove-sms-parity.js`) | each renders the baseline view and the new one in one stubbed DOM from the same modem replies and diffs the DOM: structure/labels byte-identical, only the listed value corrections differ. Migration-time tools, not CI checks — each takes its pre-slice commit as an argument (`663f989`, `dfb4810`, `24ed5ef`, `fceb6ea`, `4474436`, `812ba41`) and exits 2 when handed a revision that already contains its slice. The AT-side fixtures they share live in `scripts/lib/at-fixtures.js` (`decodeSyscfg`/`decodeC5gOption`/`decodeNrCapability` cover the radio-preference replies the same way the other decoders cover their modules), each script pinning them against the matching Rust unit tests; the SMS proof's message vectors come from the WebUI demo data and from `modules/sms::pdu`'s own septet test |
| Minified-tree smoke (`scripts/smoke-minified-luci.js`, after `scripts/minify-luci-frontend.sh`) | loads the *minified* wireless and SMS pages and drives them: expected route calls, no CLI call, the readings/dropdowns/sidebar the user actually sees, and one write path. Post-minify grepping is meaningless (esbuild renames and inlines), so the only honest check of the shipped artifact is rendering it |

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
   `sms-set`/`sms-ims` verbs run the same routes. The LuCI SMS view moved onto
   those routes as well (see the slice note below), which leaves the CLI's
   `sms-list`/`sms-info` verbs without a caller — they belong to the same
   "raw text for whoever wants to look" category as `advanced radio`, and are
   folded together in the remaining-work list. (`ca`, `qos` and the
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
   surfaces left in the WebUI are only the deliberate
   `pages/at/Terminal.tsx` console. The cell-scan panel moved next:
   `modules/cell/scan.rs` owns the command builder (manual 5.35's constraints
   and the band bitmap), the `^CELLSCAN:` line parser and the exclusive task
   behind `cell.scan_start`/`cell.scan_state`/`cell.scan_abort`, pushing
   decoded cells on `cells[]`; `modem/cellscan.ts` (the WebUI's own parser and
   command builder) is deleted, and the raw `AT^CELLSCAN*` form that still
   reaches the WS command path funnels into the same task via
   `scan::pseudo_command`, so it is a compatibility shim rather than a second
   implementation. *Open item, LuCI side:* its scan modal reads
   `mt5700m-at cellscan`'s text, and that CLI verb reaches the daemon through
   the control socket's raw `send` (a passthrough capped at ~30 s, which cannot
   carry a full-band scan) — so the modal has had nothing to render since the
   scan became asynchronous. It should call `cell.scan_start` and render the
   `cellscan` push, keeping its modal markup; the CLI verb stays byte-for-byte
   for scripts until then. The last three
   system-page cards followed: `modules/modem` owns the NR capability reads and
   writes (`modem.nr_capability`/`_set`, `^NRRCCAPQRY`/`^NRRCCAPCFG` for CA, VoNR
   and DSS with the reply-kind matching and the range rules), `modules/network`
   owns `network.syscfg`/`network.syscfg_set` (the seven-argument `^SYSCFGEX`
   write plus the CLI's `set-radio-policy` validation) and `modules/system` owns
   `system.thermal`/`system.thermal_set` (the four `^THERMLD*` reports and the
   master switch). `pages/system/Info.tsx` therefore contains **no AT command at
   all** — 35 raw call sites at the start of the session, 0 now.)
2. **FOTA**: the system page drove the whole upgrade — `ATE0`,
   `^FOTAMODE=0,1,0,1`, `^FOTAOEMDL="…"` (address validated in the page), then a
   1 s poller whose state machine read `^FOTASTATE?` / `^FOTADLQ`, resumed on
   31 and flashed on 40 — so closing the page orphaned the flow, and a reload
   could neither see nor rejoin it. `modules/system/fota.rs` is the flow now:
   one named task, `system.fota_start` / `system.fota` / `system.fota_abort`,
   the state machine and the page's own step numbers, published as
   `fota.progress`. The page renders the snapshot and keeps its markup, copy
   and flow; it no longer sends a single AT command (its version card reads
   `modem.get`, the same source as the `modem` topic it already used).
3. **CLI adapter thinning**: `api/cli.rs` still holds the per-verb text
   formatters for those capabilities; they move into the modules' `api.rs`
   `render_text` as the modules land.
4. **Dial page**: the last raw-AT page. It is read-only (LuCI owns every
   write) but built six queries itself and decoded them locally — including
   issuing `^TDCFG?` twice for two cards and re-implementing the "autodial off,
   use the NDIS session" fallback. `modules/network` now owns them
   (`network.autodial`, `network.usb_mode`, `network.interface_cfg`,
   `network.pdp_contexts`) and the page renders the objects, so the WebUI has no
   AT command outside the deliberate `/at` terminal.
5. **USSD**: the SMS page's USSD panel packed the code itself
   (`AT+CUSD=1,"<gsm7 hex>",15`) and unpacked the network's `+CUSD:` URC,
   mapping `<m>` to Chinese copy — three copies of the same handbook tables if
   the LuCI side had ever needed it. `modules/sms/ussd.rs` now owns packing,
   decoding and the copy; `sms.ussd_send`/`sms.ussd_cancel` are the routes and
   the decoded answer also goes out as the `sms.ussd` event, so no page parses
   a `+CUSD` line.
6. **Schedule (定时锁频)**: the panel read and wrote its config through a JSON
   blob smuggled inside a pseudo-AT command (`AT+SCHED=`), and the daemon port
   answered with the flat UCI map — every value a string, no `night`/`day`
   nesting — so no frontend could render it, while a save wrote top-level UCI
   keys that do not exist. `modules/network/schedule.rs` owns the DTO and its
   UCI mapping, `network.schedule_get`/`schedule_set` are the routes, and
   `AT+SCHED?`/`AT+SCHED=` remain as aliases to them for the CLI and LuCI.
7. **LuCI scan modal**: the modal showed the scan result from one blocking
   `mt5700m-at cellscan`; once the scan became a minutes-long exclusive task the
   control socket's timeout made that call answer with the immediate
   acknowledgement, so the frequency-scan card came back empty. The CLI verb now
   prints the two fast sections, starts the scan when none runs and prints the
   `^CELLSCAN` lines from `cell.scan_result` (the scan task caches its reply
   text, TTL 30 min); `cellscan-result` is the poll the modal uses, and the
   modal's markup, copy and flow are unchanged — it just refreshes itself when
   the scan finishes.
8. **Carrier-aggregation card data feed**: the WebUI's `pages/network/Info.tsx`
   renders its carrier list and per-carrier signal from `cell.carrierInfo`,
   `secondaryNR`/`secondaryLTE` — and when the AT-era feed was removed nothing
   assigned any of them, so the card rendered empty even though the page kept
   the merge helpers. Fixed by moving the missing decode into the module that
   already owns those commands: `modules/ca` parses `^CASCELLINFO?` **once**
   (the carrier list and the secondary list now share one field map) and
   `^MONSSC` per-cell (hex PCI, the manual's invalid values, the ±8 descale
   heuristic, `<MEASTYPE>`), and `ca.get` answers a `secondary` array next to
   `carriers`. The page fetches `ca.get` (cache-first, `refresh` on demand),
   subscribes the `ca` topic and maps the domain model with `modem/ca.ts`
   (presentation only; `modem/carrier.ts` and its two parsers are gone).
9. **Frontend AT removal**:
   * WebUI — display flows through the daemon's topic cache
     (`services/stateCache.ts` + `useSharedStateTopic`) and every action goes
     through a route; the only remaining AT builder is
     `pages/at/Terminal.tsx`, the deliberate raw-AT console (user-facing
     diagnostic tool, admin-only). `modem/*.ts` holds types, display tables and
     merge helpers only.
   * LuCI — `api.js` gained `route(name, params)` (the `api.<route>` command
     path over `mt5700.at`, documented in `api-contract.md`), and the wireless
     page's whole diagnostics block moved over:
     * the SSB panel and the "NR neighbour cells" row read `beam.ssb`
       (`parser.js`'s `parseNrsSbid` — the second, JS copy of the `^NRSSBID`
       offsets — is deleted);
     * the two modulation rows read `modem.mcs`. The route flattens all `^MCS`
       lines into one carrier list, so the module's `McsCarrier` carries the
       reply's own `group` and `rat`, and the page rebuilds the exact blocks it
       printed before ("NR Carrier 1", "LTE Carrier 1", …) —
       `parser.js`'s `parseMcsSection` is deleted, the modulation table
       (`mcsModulation`) stays, because it is display;
     * NR PUSCH/PUCCH power and transmit frequency read `modem.nr_txpower`,
       the QoS class `qos.get`, the secondary-carrier counts `ca.get`
       (`lte_secondary_count` / `secondary_connection_count`), data
       registration `registration.get`, EN-DC `modem.endc`, IMS registration
       the new `network.ims` route (`+CIREG`, which no module decoded before).
     * Parity was verified with a node harness that runs the *old* view and the
       *new* one in the same stubbed DOM and compares every rendered row
       (12 rows on the mock modem, plus nine corner cases: single- and
       multi-carrier MCS replies, an empty reply, a missing `+CGEQOSRDP`, the
       999/0 transmit-power sentinels, a missing `^CASCELLINFO`, `^MONSSC:
       NONE`, a missing `+C5GREG`): byte-identical in every case, MCS labels
       included. Two differences, both in the failure path and both
       intentional: when `+CIREG` or `^LENDC` does not answer, the old code
       printed "Not registered" / "Disabled" from a placeholder row the text
       matcher invented, and the page now shows the row's own `--` (the same
       placeholder every other empty row uses).
     * the neighbour cards followed, in all three places they are drawn: the
       diagnostics section, the SSB panel and the scan modal's two grids now
       render `cell.neighbors` (`parser.js`'s `parseMonnc` — the JS copy of the
       `^MONNC` layout — is deleted). The ARFCN→band table left the frontend
       too: it is `core::radio::arfcn_to_band` (shared by `modules::cell` and
       `modules::beam`, both of which now publish `band`), and the page only
       maps a band number to its label (`B3`/`n78`). `mt5700m-at cellscan` no
       longer prints a neighbour section (it was the modal's only consumer, and
       printing it meant asking `AT^MONNC` twice); `radio-diagnostics` still has
       it for humans. Parity: `scripts/prove-neighbors-parity.js` renders the
       HEAD view and the new one in the same stubbed DOM from the same modem
       replies — structure, labels and row count byte-identical, and the four
       value slots that differ are exactly the listed corrections (hex PCI
       `1DC`/`40` → decimal `476`/`64`, and n78 cells that the old JS table
       printed as bare `NR` because it only covered up to 3 GHz). Re-runnable
       after the commit: `node scripts/prove-neighbors-parity.js 663f989`
       (the pre-slice commit is the baseline; without it the script says so and
       exits 2).
     * the frequency lock followed, both directions: the two lock-status rows
       and the lock panel's prefill read `network.lock_get`, and every write
       (the panel's Review-and-apply and the neighbour cards' Lock button) goes
       through `network.lock_apply` with typed `items` — the CLI's positional
       `['lock', rat, type, bands, arfcns, scs, pcis]` arguments are gone from
       the frontend, and with them two defects the harness pinned: the NR ARFCN
       card passed an empty SCS slot (the CLI's builder rejects it) and the LTE
       card passed the PCI in the ignored 7th slot, so `pcis` was empty. The
       panel's prefill was also empty on real hardware, because
       `collectFreqLock` required every reply row to repeat the `^…FREQLOCK:`
       prefix while the firmware prints it once — the route hands over the
       decoded items instead (PCI back to decimal, which is what a write needs).
       `verify: true` keeps the CLI flow's outcome reporting (the module polls
       the query, the page shows the message). `parser.js`'s `collectFreqLock`
       and `parseLockData` — the second and third JS copies of the lock layout —
       are deleted, and `parser.arfcnToBand`'s last caller is now only the scan
       modal's serving cell. `scripts/prove-lock-parity.js dfb4810` proves it:
       panel structure/copy byte-identical, prefill equal to the domain values,
       items one-to-one with the form, and the four outcome paths (success,
       modem rejection, verification failure, transport failure) producing the
       same notifications with success waiting 2.5 s before the reload.
       * The CLI's `network` verb no longer prints the `LTE lock` / `NR lock`
       sections (the panel was their only consumer, and printing them asked the
       modem the same two questions again); `mt5700m-at lock` still writes,
       `status` still reports `lte_lock=`/`nr_lock=`, and `radio-diagnostics`
       keeps the raw lines. ucode's timeout budget grew a named list for
       migrated write routes (`api.network.lock_apply` → 25 s, the same budget
       the `lock` verb gets) instead of the read-class 12 s default.
     * the wireless page's status block followed (the last part of that page
       that still sliced the `mt5700m-at network` frame): the signal gauges read
       `signal.get`, the serving-cell rows `cell.get`, registration
       `registration.get`, the operator `network.get`, the RRC row the new
       `network.rrc` route (`^RRCSTAT?`, which no module decoded before) and the
       temperature gauge `system.temperature`'s `peak`. The gauges' per-RAT
       metric sets are unchanged (NR: RSRP/RSRQ/SINR, LTE: RSRP/RSRQ/RSSI,
       WCDMA: RSCP/RXLEV/ECIO) but their numbers now come from the one `^HCSQ`
       decoder in `modules::signal` instead of `^MONSC`'s fields 7..10 — the two
       are the same measurement (MONSC reports dBm, HCSQ the index that maps
       back to it), so the reading the user sees is the one the WebUI and the
       `status` page already show. `parser.js`'s `parseServingCell` (the second
       JS copy of the `^MONSC` layout) and `api.js`'s `atNetwork` (the CLI-frame
       call nobody else made) are deleted; the page's `mt-row` set is otherwise
       byte-identical, and the "Technical details" collapsible — which used to
       dump the whole AT frame — now dumps the route payloads it consumed, so no
       AT reply text reaches the frontend at all. Parity:
       `scripts/prove-network-parity.js` renders the old view and the new one in
       the same stubbed DOM from one consistent modem report and compares every
       row: 12 rows, the only value differences are the intended hex→decimal
       normalisation of `PCI`/`Cell ID`/`TAC / LAC` (the same correction the
       neighbour cards got), every gauge reading, label, copy and card title
       byte-identical. Re-runnable after the commit:
       `node scripts/prove-network-parity.js 24ed5ef` (the pre-slice commit is
       the baseline; without it the script says so and exits 2). The
       temperature topic now carries `peak`/`peak_sensor` next to the twelve
       sensors and `average`: the dashboard's gauge has always rendered
       `average` (`status.js`) and the WebUI's cards the individual sensors, so
       the three surfaces still read three different fields of the same
       reading — collapsing them onto one is a UI decision, not a decode one,
       and is left to the owner (the decode itself is single now: one
       `TemperatureState`, one `peak()`).
     * the scan modal followed. Its serving-cell card read the `^MONSC` text and
       guessed the band with `parser.arfcnToBand`, its measurement bars came from
       `^MONSC`'s fields 7..10, the scan state came from the
       `mt5700m-at cellscan-result` text JSON and starting a scan was a side
       effect of the `cellscan` verb. Now: `cell.get` (band from
       `core::radio::arfcn_to_band`, PCI/CID decimal) + `signal.get` for the
       bars, `cell.scan_result` for the state, `cell.scan_start` when nothing has
       been scanned yet — the last three mirroring what the CLI verb did
       (present a finished scan rather than starting a new one).
       `parser.parseMonsc` (the third JS copy of the `^MONSC` layout — `cell.get`
       and `cell.neighbors` no longer need it either) and `parser.arfcnToBand`
       (the band table that only covered up to 3 GHz: for an n78 ARFCN it did the
       0–3 GHz linear conversion, landed outside every range and printed
       `NR · NR`) are deleted, as are `api.atCellscan`/`api.atCellscanResult`;
       the wireless page was left with a single CLI call (`atRadio`, its
       radio-preference block), which the next slice removed as well — see
       below.
       Parity: `scripts/prove-cellscan-parity.js fceb6ea` drives the real
       «Cell Scan → Continue» path on both revisions — the modal title, both
       neighbour cards, the labels and the copy are byte-identical and all four
       differing lines are the intended value corrections (the `NR · NR` →
       `NR · n78` band, the decimal PCI/CID, and the SINR bar that `^MONSC`'s NR
       layout does not carry, so the old card drew `--`); it also pins the
       no-`band` fallback, which is the same `NR · NR` the old table produced.
       The modal's **"Frequency scan" card is deleted, not restored**: it has
       never rendered (the `parser.section` prefix mismatch described below), so
       removing the text path removes dead code, not UI. Bringing the scan
       results back is a UI decision — `cell.scan_result` already carries the
       decoded `cells` and the raw `^CELLSCAN` text whenever someone decides what
       the card should show.
     * the wireless page's radio-preference block closed the page: the
       access-technology rows read `network.syscfg` (`^SYSCFGEX?`),
       `network.c5goption` (`^C5GOPTION?`) and `modem.nr_capability`
       (`^NRRCCAPQRY` 3/2/5), and its five buttons write
       `network.syscfg_set`, `network.c5goption_set` and
       `modem.nr_capability_set` — the same three read routes and three write
       routes `pages/network/Settings.tsx` and `pages/system/Info.tsx` already
       call, so the two frontends edit the same contract. The five
       `advanced-set radio-policy|5g-access|carrier-aggregation|vonr|dss`
       invocations are gone, which makes `mt5700m-at advanced radio` dead;
       `api.js`'s `atRadio` (the page's last CLI call, and with it the page's
       last text frame) and `parser.js`'s `matchValues` (whose only caller was
       this block) are deleted. The preset→triple mapping lives on the page
       (`1,0,1` = Option 2, `0,1,0` = Option 3, else `1,1,1`), and `ca` is sent
       as the boolean the route validates rather than the CLI's `'1'`/`'0'`.
       Missing fields keep the old fallbacks (`080302`/`3FFFFFFF`/`1`/`2`/
       `7FFFFFFFFFFFFFFF`) because `SysCfgState` omits unread fields, an
       incomplete `^C5GOPTION` triple still lands on Option 2 + 3, and a
       half-read `^NRRCCAPQRY` set still leaves VoNR/DSS on their previous
       defaults. Two warning banners disappear with the frames that filled them:
       the page's top banner and the 5G card's both rendered the `radio` frame's
       `stderr` (`status.stderr`/`radioSettings.stderr`), and a read route has no
       stderr to show — `api.route()` is the deliberate "never reject, missing
       data renders blank" contract the other ten reads on this page already
       use, so a failed read now shows an empty control or the default value
       instead of a raw CLI error box. ucode's migrated-write budget list gains
       the three routes (`c5goption_set` cycles airplane mode like the old
       `advanced-set 5g-access`, so it needs the 25 s budget; the other two are
       multi-command configuration writes). Parity:
       `scripts/prove-radio-parity.js 4474436` renders both revisions from the
       same replies — the card is byte-identical in eight shapes (all fields
       missing, empty `^SYSCFGEX` fields, roam/service 0, the three access-mode
       triples, an incomplete triple, CA off with VoNR FR2) and all five write
       paths are driven through the real modal: the old CLI argv and the new
       typed params carry the same values, the same notification text and the
       same 900 ms reload. The wireless page now makes no CLI call at all —
       asserted on the call surface, not by grepping the source.
     * the SMS page followed, and with it the frontend's last PDU decoder. The
       page now reads `sms.list` (PDU decode, GSM-7/UCS-2, UDH concatenation and
       part merging all happen in `modules/sms`) and `sms.status`
       (`+CPMS`/`+CSCA`/`^IMSSWITCH`), and writes `sms.send`/`sms.delete`/
       `sms.clear_all`/`sms.center_set`/`sms.storage_set`/`sms.ims_set`. Both
       reads go through `api.routeCall` rather than `api.route`, deliberately:
       the message list *is* the page, so a failed read has to be reported —
       rendering "No messages yet." for a modem that never answered would claim
       an empty inbox. That keeps the page's two `alert-message warning` banners
       (their copy is unchanged; the text is the backend's message, as it was
       when the same banner showed the CLI frame's stderr). Deleted with the
       frames: `api.js`'s `atSmsList`/`atSmsInfo` and `parser.js`'s
       `parseMessages`/`parseInfo`/`decodePdu`/`decodeGsm7`/`decodeUcs2`/
       `swapDigits`; the `groupMessages` that stays is a pure display grouping.
       ucode's migrated-write budget list became `[route, seconds]` pairs so
       every write keeps the budget its CLI verb had (`sms.send`/`clear_all`/
       `ims_set` 60 s, the rest 25 s). One pre-existing dead path is left in
       place and flagged rather than deleted: `renderPage`'s `deleteMessage`
       dialog has no button wired to it, so the page cannot reach it — its one
       line moved onto `sms.delete` with the rest, and deciding whether a
       per-message delete button should exist is a UI decision.
       Parity: `scripts/prove-sms-parity.js 812ba41` renders both revisions from
       one set of modem replies — seven shapes (five received messages over
       three differing `+CPMS` planes, the WebUI demo data's same-plane storage,
       IMS off, a status read that fails, an empty inbox, and single-conversation
       UCS-2 and GSM-7 inboxes), the settings dialog (prefill + the three writes
       + the 1500 ms reload), the send flow with its local sent-history write,
       the clear flow, and both failure paths. The message fixtures are the
       WebUI's demo vectors (`mockAT.ts` RECEIVED_SMS) and the GSM-7 one is built
       from `modules/sms::pdu`'s own `pack_septets(["hello"])` test vector, so the
       proof compares the backend's decode against the old JS decode rather than
       against a fixture copied from either side.
     * Still on text frames, to be migrated next: `parser.section`/`pick` for
       the settings-page and dial-page rows, and the `status`/`system` pages,
       which still call the CLI verbs (`status` also feeds the dashboard's
       `parser.signalQuality`/`operatorInfo` rows). `parser.js` (`section`,
       `pick`) is deleted when the last of them moves. The dashboard already
       reads the daemon cache via `api.cachedSnapshot()`.
     * Defect found while proving the above, **not** changed (it is a UI
       change, so it needs a decision): the scan modal's "Frequency scan" card
       never renders, on HEAD or now. `parser.section()` matches a prefix
       `'===== <label>:'`, and the modal passes `'Frequency scan: AT^CELLSCAN'`
       while `cli.rs` prints `===== Frequency scan: AT^CELLSCAN =====` — no
       match, so the card (and its "+CME ERROR: 3" note) is dead. The two
       other labels that include the AT command survive only because their
       callers pass `|| raw`. The modal migration deleted the card together with
       the text path instead of restoring it — it had never been visible, so the
       rendered UI is unchanged, and what the card should show is a UI decision
       (the decoded `cells` or the raw `^CELLSCAN` text). The same pass removed
       the modal's `mt5700m-at cellscan` call itself, which the control socket's
       timeout cannot carry for a full-band scan.
10. **Fold `mt5700m-traffic` into `modules/traffic`** (last non-Rust business
   process) and the dialing glue of `mt5700m-manager` into `modules/network`
   actions.
11. **Split the two god files** (`daemon.rs` ~1.9k lines, `api/cli.rs` ~2.4k):
   `daemon.rs` → `transport/{ws_server,rpc_server,control_server}` +
   `api/rpc.rs`; `api/cli.rs` → one `render_text` per module + a small verb
   table. Both are now pure wiring/adapters, so the split is mechanical.
12. **Fold the CLI's radio write verbs onto the routes.** With LuCI's
   radio-preference block migrated, `advanced-set
   radio-policy|5g-access|carrier-aggregation|vonr|dss` have no frontend
   caller, yet each still builds its own AT string (`AT^SYSCFGEX=`,
   `AT+CFUN=0` + `AT^C5GOPTION=`, `AT^NRRCCAPCFG=`) while `modules::network`/
   `modules::modem` build the same commands for `network.syscfg_set`,
   `network.c5goption_set` and `modem.nr_capability_set` — the same write
   implemented twice inside the backend. The SMS verbs already show the target
   shape (`run_api(settings, "api.sms.send", …)`: the CLI is a client of the
   same API as both frontends, no second writer on the port). Folding these
   five means translating argv → route params (keeping each verb's own
   validation and `EXIT_USAGE` argc checks) and accepting two CLI-visible
   changes the SMS verbs already made: a route-backed verb prints nothing on
   success, and a module/daemon failure exits 1 instead of the AT error code.
   Rust-only change → CI is the only compiler, so it wants its own slice with
   unit tests over the argv → params mapping.
   `sms-list`/`sms-info` are in the same state (no frontend caller left after
   the SMS slice) and can be dropped outright or reformatted as one
   `render_text` from `modules/sms` — either way they belong to this batch.
   The read side stays: `advanced radio` / `radio-diagnostics` are the CLI's
   raw-AT diagnostic views (the same reason `radio-diagnostics` was kept).

### Inventory of the remaining LuCI pages (recon 2026-10-07)

Checked against the route table (`modules/*/api.rs`) and the state structs, so
this is what the next slices start from rather than a guess:

| Page | Read it still slices | Write verbs it still uses | Covered by existing routes |
| ---- | -------------------- | ------------------------- | -------------------------- |
| `status.js` | `status`, `advanced session` | — | `signal.get`, `cell.get`, `registration.get`, `network.get`, `modem.get`, `traffic.get`, `network.pdp` (`+CGPADDR`), `network.dhcp`, `qos.get`, `ca.get`, `modem.txpower`/`nr_txpower`/`endc`; the dashboard already reads the daemon cache for its topics |
| `system.js` | `system` | `sim-pin`, `advanced-set thermal-thresholds`/`thermal-log`, `factory-reset` | reads: `modem.get` (manufacturer/model/revision/imei), `sim.get` (status/iccid/imsi/number/slot/hotplug), `system.temperature`, `system.thermal`, `system.fota`; writes: `sim.pin_apply`, `system.thermal_set`, `system.factory_reset`, `modem.imei_set`, `system.fota_start`/`fota_abort` |
| `connection.js` | `advanced connection-settings`, `advanced session` | `pdp-set`, `advanced-set autodial`/`direct-ip`/`postroute`/`dmz`/`pdp-state`/`pdp-remove` | reads: `network.autodial`, `network.interface_cfg`, `network.pdp_contexts`, `traffic.*`, `network.dhcp` |
| `advanced.js` | `advanced hardware` | `advanced-set usb-mode`/`pcie-controller`/`nic-speed`/`interface-mode`/`sim-hotplug`/`thermal` | reads: `network.usb_mode`, `network.interface_cfg`, `system.device_control` (nic_rate/power_control), `sim.get` (hotplug), `sim.slot`, `system.thermal`; writes: `system.nic_rate_set`, `system.power_control_set`, `sim.hotplug_set`, `sim.slot_set`, `system.thermal_set` |
| `terminal.js` | `command` | — | **kept deliberately**: the terminal *is* a raw AT console; it is the one place a user types AT on purpose |

Gaps that need a **new backend route** before those pages can move (each is one
`service` function + one `Route` + a JSON shape, following `module-guide.md`):

* `system.js`: the `^VERSION` fields (build date / software / hardware), network
  time (`^NWTIME`), the LED switch (`^LEDSWITCH` — `system.device_control`
  currently carries only `nic_rate`/`power_control`) and the SIM activation
  power (`^HVSST`).
* `connection.js`: `^SETAUTODIAL` write, the forwarding trio
  (`direct-ip`/`postroute`/`dmz`), and the PDP writes (`pdp-set`/`pdp-state`/
  `pdp-remove`). `network.pdp` today is only the `+CGPADDR` read.
* `advanced.js`: the `usb-mode` and `interface-mode` writes (`^SETMODE` /
  `^TDCFG`).

Those are Rust slices (CI is the only compiler), which is why the LuCI batch
splits page-by-page behind them; the two pages whose fields are already covered
(`status.js`, and `system.js`'s SIM/thermal/FOTA half) can move first.

### Status page: the CLI status frame is gone (slice, 2026-10-07)

`view/mt5700m/status.js` was the page the inventory called "already covered".
Done here, on baseline `8a8c501`:

* the **detail frame** no longer calls `mt5700m-at status` (a cached key=value
  frame *plus* four live AT queries *plus* a reachability probe, all through
  `client::at_cmd` in the CLI process — the second AT owner this architecture
  forbids). The four dashboard rows it fed are now routes:
  `network.pdp_contexts` for the cid-1 APN (`+CGDCONT?`/`+CGACT?`, the same
  command the CLI read), `qos.get` for QCI + subscribed rate (kbps → Mbps with
  the CLI's `{:.1}`), `sim.number` for `+CNUM`;
* `usb_state` comes from the `usb` topic (the presence collector already
  classifies 3301/3302/3303; `present=false` maps to `absent`, exactly what the
  CLI printed when `mt5700m_usb_info()` found nothing);
* `api.js` lost the `atStatus` verb. `advanced session` is still a CLI call —
  the address card is the next slice;
* backend: `modules/sim/parser.rs::cnum_number_state` (+CME ERROR 22 *and* its
  CMEE=2 text "not found" → `not_stored`, added to `CME_TEXTS`),
  `SimState::number_state` (JSON `numberState`), `sim.number` keeps a rejected
  `+CNUM` reply instead of dropping it (that reply *is* the answer, same pattern
  as `+CPIN?`), and `parse_cnum` now normalizes/validates the field (strip
  quotes/spaces, digits with optional `+`, length ≥ 5) — the rule the CLI had
  re-implemented inline, now single-sourced in the module.

Two defects in the old detail frame, both fixed by the rewrite and both pinned
by `scripts/prove-status-parity.js`:

1. `refreshDetail` passed `native.stdout` (**a string**) as `mergeStatusLines`'s
   `overrides`, whose body does `(overrides || []).forEach` → guaranteed
   `TypeError`, caught by the chain's `.catch`. So the CLI frame never merged at
   all: the deployed page showed a permanent "Some modem details could not be
   refreshed. … (overrides || []).forEach is not a function" banner and the
   APN / QCI / subscribed-rate / phone rows were **always blank**.
2. That exception fired before `state.sessionDetail` was assigned, so the
   Mobile IP card (`advanced session`) was **always empty** on the deployed
   page too. The route-based version sets the session frame independently, so
   the card renders.

Both are the UI the cards were built for, not new UI: nothing was added to the
DOM except the values the page always meant to render. Deliberate, counted
difference (see `NUM_PLUS` in the proof): the phone number loses the CLI's `+`
prefix, because the backend's domain numbers are bare digits (same as the
WebUI's `normalizePhoneNumber`). Two more recording-only notes:

* the CLI published `network.sysmode` under the key `network_mode`, and
  `parseStatus` prefers it over `sysmode_detail` — following that would have
  changed the "Network Mode" cell from `5G SA` (the deployed value, and the one
  the WebUI derives from the same topic) to the coarse `NR5G`. The snapshot
  mapping therefore keeps `sysmode`/`sysmode_detail` and the proof asserts the
  cell is byte-identical to the deployed page.
* temperature: `mergeStatusLines` was last-write-wins, so the snapshot's raw
  float always overrode the CLI's `round()`. The page has shown `45.1`, not
  `45`, since long before this slice — the mapping keeps it.
* carrier frequencies: the CLI derived `dl_freq` with `nr_arfcn_to_mhz()`
  (LTE's ARFCN rule, applied to NR ARFCNs → wrong by orders of magnitude) and
  the page's own `carrier_1` line overrode it in the merge, so it was never
  visible for the single serving carrier. The page's `carrier_1` now leaves the
  frequency columns blank; real multi-carrier MHz values come from the `ca`
  topic when the carrier card grows a multi-carrier list.

Remaining CLI call sites afterwards: `status.js` 1 (`advanced session`),
`system.js` 5, `connection.js` 3 reads + its write verbs, `advanced.js` 1,
`terminal.js` 1 (deliberate).

Harness work in this slice: `scripts/lib/luci-stub.js` grew
`querySelector(All)` / `parentNode` / `replaceChild` / `removeChild` and
`L.url`, because `updateRegions` does a real incremental DOM replacement —
without them the detail frame's update path could not run in a proof at all.
`scripts/prove-status-parity.js` (baseline `8a8c501`) compares eight shapes
three ways: deployed baseline vs new with the detail routes stubbed to `null`
(body must be byte-identical), the CLI frame's values vs the routes' values
(patched baseline), and the two regions that legitimately change
(`alerts`, `address`) asserted by content. `scripts/smoke-minified-luci.js`
now renders the status page too (16 checks; minified tree 297 500 → 186 254 B).

### The session frame is gone: `network.session` (slice, 2026-10-07)

Second half of the same job, on baseline `c0268e0`. `advanced session` — the CLI
verb that dumped eight AT replies (`^NDISSTATQRY?`, `^DHCP?`, `^DHCPV6?`,
`^IPV6CAP?`, `+CGPADDR`, `^DSFLOWQRY`, `^CGMTU=1`, `^DCONNSTAT?`) as text for
`parser.parseSession()` to regex — is now a route:

* `network.session` (`modules/network/{commands,parser,state,service,api}.rs`)
  returns the decoded snapshot: `{ipv4:{connected,address,gateway,dns[]},
  ipv6:{connected,address,dns[]}, capability, mtu, maximum_down, maximum_up,
  flow:{current_duration,current_tx,current_rx,total_duration,total_tx,total_rx},
  sessions:[{cid,apn,ipv4,ipv6,type,ethernet}]}`. Both surface's readers
  (overview "Mobile IP" card, connection page panel) call it — one decoder.
* `network.flow_clear` (`AT^DSFLOWCLR`) replaces the `flow-clear` CLI verb
  behind the "Clear counters" button; the button now uses `c.confirmRoute`.
* `api.js` lost `atSession`; `parser.js` lost the whole text decoder —
  `parseSession`, `csvValues`, `hexIPv4` (the smallest helpers existed only to
  read that frame) — and gained `sessionInfo(payload)`, which keeps the
  page-facing field names (`ipv4Connected`, `maximumDown`, …) so the rendering
  code did not move. The three remaining mappings there are display choices
  (capability code → localized label, missing MTU → "Network default", DNS list
  → " · " join).
* `DhcpLease` kept the reply's two trailing fields (`maximum_down`/`maximum_up`,
  verbatim — the CLI page rendered them through its rate formatter, so decoding
  them in the module would have changed what it shows).

Two decoders got stricter/correcter while moving, both asserted by unit tests:

* `parse_ipv6cap` now also reads the hex form. The WebUI has always rendered
  `0x0B` ("separate APNs") but the field was parsed as decimal only, so `0B`
  fell through as "no answer" and both pages lost a value they were designed to
  show.
* `parse_dconnstat` normalizes the full-width separators/quotes (`，`, `“”`)
  the firmware sometimes sends — the frontend's regex tolerated them, a naive
  comma split would not have.

An earlier plan in this series had the connection page drop `form.Map` and build
the dialing form by hand. It was dropped: `form.Map` is LuCI's own UCI form
machinery (the same thing `settings.js` uses), the page's UCI reads/writes are
host configuration rather than modem business logic, and hand-reproducing CBI's
markup/CSS byte-for-byte is a UI-change risk with no architectural payoff. The
page keeps the framework form; only the modem data path moved to routes.

The CLI verbs themselves stay. `mt5700m-at status`, `advanced session` and
`flow-clear` are documented tools (`README.md` uses `status` in its diagnostic
recipes) and they are **not** a second implementation of anything: `status`
renders the daemon's StateCache in the CLI's own `key=value` shape, and the
`advanced <group>` dump is raw reply text, not a parser. Deleting them would be
a user-visible loss with no architectural gain — what the refactor forbids is a
*frontend* owning a second decoder, and that is what this slice removed. (Their
AT reads go through the daemon channel, i.e. the one scheduler; the only
remaining "opens the port itself" path is the CLI's no-daemon fallback, which is
the next candidate.)

`scripts/prove-connection-parity.js` (baseline `c0268e0`, 30 checks) renders the
connection page and compares five shapes: the session fixture
(`scripts/lib/at-fixtures.js::sessionFacts`) generates **both** the CLI text
frame and the route payload from one object, so "identical render" means the
module's decoding equals what the frontend used to regex out of the dump —
covering the full `^NDISSTATQRY` rule, the empty-NDIS fallback ("has an address
⇒ connected"), IPv4-only, the `0B` capability case, and a session read that
fails entirely (both sides then render the same empty card — no new banner, the
CLI failure path did not have one either). It also pins the write path
(`flow-clear` → `network.flow_clear`) and the 900 ms reload.

With this, the overview page has **zero** CLI calls and the connection page is
down to one (`advanced connection-settings`, which is the read half of the
dialing settings the page still writes through `advanced-set`); the remaining
CLI call sites are `system.js` 1, `advanced.js` 1, `connection.js` 1,
`terminal.js` 1 (deliberate).

### System page writes move to routes (slice, 2026-10-08)

The system page's six remaining CLI **writes** now dispatch through the module
registry. The 22-section read frame (`mt5700m-at system`) is untouched, and
seven writes deliberately stay on CLI verbs — see below.

| old CLI verb | route | equivalence |
| ------------ | ----- | ----------- |
| `airplane <0\|1>` | `network.radio_set {airplane}` | same `AT+CFUN`; the page's `0\|1` becomes the domain boolean |
| `advanced-set sim-slot <v>` | `sim.slot_set {slot}` | **deliberate difference** — the module runs the vendor's full switch sequence, the CLI only sent `AT^SCICHG` |
| `set-imei <v>` | `modem.imei_set {imei}` | same `AT^PHYNUM=IMEI`; the 15-digit rule lives in the module, once |
| `restart` | `modem.reset` | same `AT^RESET` (`commands::RESET`) |
| `sim-pin <op> a1 a2` | `sim.pin_apply {operation,pin,newPin}` | five operations map one-to-one; `newPin` is `""` where the UI hides the field, exactly what the CLI passed |
| `factory-reset` | `system.factory_reset` | `AT&F` vs the CLI's `AT&F0` — `&F` defaults to profile 0, so no behaviour change |

`components.js` already factored the write-confirmation modal into
`confirmAction`, so this slice did the same for the `danger` + custom-delay
variant: `runConfirmedAction` is now the only implementation, with
`runConfirmed` (CLI verbs) and the new `runConfirmedRoute` (routes) as its two
thin callers. The modal's DOM, wording, button order and the
recovery-delay behaviours cannot fork.

Seven writes stayed on CLI verbs after this slice: four of them (`advanced-set
led`, `advanced-set sim-activation`, `advanced-set thermal-thresholds`,
`advanced-set thermal-log`) because the backend had **no** write route for
`^LEDSWITCH`, `^HVSST=`, `^THERMLDAUTOPARA=` or `^THERMLDLOGSW=` at the time,
plus `fota-start` / `fota-resume` / `fota-upgrade`, where the module's shape is
"start a task, then observe it" while the page drives a three-step
download/resume/install flow; converting that is a product decision, not a
refactor step. The first four moved a slice later — see
[below]((#system-page-the-other-four-writes-move-to-routes-slice-2026-10-08))
once the backend had the four groups of commands; only the FOTA three still
intentionally hold the CLI line.

`scripts/prove-system-parity.js` (baseline tag `pre-system-route`, 34 checks)
renders six read frames — a full frame, SIM needs PIN, number not stored,
FOTA install-complete, all thermal sections missing, and airplane mode — and
requires every line byte-identical, because this slice did not touch parsing.
The twelve mapping checks pin **both** sides at once: what the old page put on
the command line *and* what the new one sends, so a route with wrong parameters
fails even though nothing else changed. Seven further checks assert the
retained verbs were **not** migrated.

`scripts/smoke-minified-luci.js` gained a system-page block (22 → 28 checks):
it drives the four hazard-free writes (airplane, SIM slot, restart, factory
reset) on the *minified* tree and asserts exactly one `routeCall` each.

Note for whoever takes the read frame: `pre-system-route` is a **tag**, not a
hard-coded SHA. The other proofs default to a SHA of their pre-slice commit,
which cannot survive a fresh clone in a sandbox — the offset in
`scripts/lib/at-fixtures.js` (`SYSTEM_FACTS` / `systemCliFrame`) now feeds both
this proof and the smoke test, so the two cannot drift.

### System page: the other four writes move to routes (slice, 2026-10-08)

The backend had no route to carry these four when the previous slice shipped,
so they were left on CLI verbs with the rest. Both halves landed since — four
new route pairs on the backend (`system.led` / `system.led_set`,
`system.network_time`, `system.thermal_thresholds_set` / `system.thermal_log_set`,
`sim.activation` / `sim.activation_set`) and this slice — which moves the last
four *simple* writes off verbs. The three FOTA writes remain, unchanged in
reason: the module answers "task plus observation", the UI drives a flow, and
closing that gap changes interaction rather than transport.

| old CLI verb | route | equivalence |
| ------------ | ----- | ----------- |
| `advanced-set led <0\|1>` | `system.led_set {enabled}` | same `AT^LEDSWITCH=<0\|1>`; the select's string becomes a boolean at the boundary |
| `advanced-set sim-activation <0\|1>` | `sim.activation_set {active}` | same `AT^HVSST=1,<0\|1>`; now built by the same constructor that brackets a slot switch |
| `advanced-set thermal-thresholds <9 values>` | `system.thermal_thresholds_set {thresholds}` | same `AT^THERMLDAUTOPARA=<9 values>` |
| `advanced-set thermal-log <s> <f>` | `system.thermal_log_set {serial,file}` | same `AT^THERMLDLOGSW=<s>,<f>` |

Two things were deliberately **not** changed:

* **The page keeps its own pre-submit validation** (the 0–150 °C / ladder check
  and its two i18n strings). That is the precedent `sim.pin_apply` set — the
  copy shown is UI, the module owns the modem's rule and re-checks it, and the
  route is not reachable with a table the modem would refuse.
* **The thermal modal still fires both writes as one `Promise.all`**, so its
  "Thermal settings saved." + reload behaviour is byte-for-byte what it was.

`scripts/prove-system-parity.js` grows from 34 to 39 checks against the same
`pre-system-route` baseline: the four verb-disappearance checks moved into group
B, group C gained LED (both positions), SIM activation (both states, driven
through the `^HVSST` second field) and the thermal pair, and group D shrank from
seven retained verbs to three. Group C still pins **both** sides per case — old
argv *and* new params — so a mistyped parameter fails rather than silently
diverging. `scripts/smoke-minified-luci.js` (28 → 31 checks) drives the new
writes on the minified tree, including the two-call thermal modal.
