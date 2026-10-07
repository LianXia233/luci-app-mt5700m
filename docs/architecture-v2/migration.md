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
| LuCI migration parity (`scripts/prove-neighbors-parity.js`, `prove-lock-parity.js`, `prove-network-parity.js`, `prove-cellscan-parity.js`, `prove-radio-parity.js`) | each renders the baseline view and the new one in one stubbed DOM from the same modem replies and diffs the DOM: structure/labels byte-identical, only the listed value corrections differ. Migration-time tools, not CI checks — each takes its pre-slice commit as an argument (`663f989`, `dfb4810`, `24ed5ef`, `fceb6ea`, `4474436`) and exits 2 when handed a revision that already contains its slice. The AT-side fixtures they share live in `scripts/lib/at-fixtures.js` (`decodeSyscfg`/`decodeC5gOption`/`decodeNrCapability` cover the radio-preference replies the same way the other decoders cover their modules), each script pinning them against the matching Rust unit tests |

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
     * Still on text frames, to be migrated next: `parser.section`/`pick` for
       the settings-page and dial-page rows, and the `status`/`sms`/`system`
       pages, which still call the CLI verbs. `parser.js` (`section`, `pick`)
       is deleted when the last of them moves. The dashboard already reads the
       daemon cache via `api.cachedSnapshot()`.
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
   The read side stays: `advanced radio` / `radio-diagnostics` are the CLI's
   raw-AT diagnostic views (the same reason `radio-diagnostics` was kept).
