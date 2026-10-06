# API contract

The API is the boundary. Frontends know route names and domain JSON; they know
nothing about AT commands, serial ports, the scheduler, cache keys or parsers.

## 1. Routes

Registered by modules in `api/registry.rs`; dispatched identically from every
transport. `*.get` answers cache-first and performs at most one bounded refresh
when the topic is still cold; `*.cached` never touches the modem.

Two kinds of route, declared in the table itself (`Route::display` /
`Route::on_demand`):

* **display** (`signal.get`, `system.temperature`, …) — every page load calls
  these, so they must answer with domain JSON even when the modem is missing or
  busy; a failed refresh degrades to the previous value.
* **on-demand** (`sim.number`, `network.pdp`, `network.dhcp`, `modem.mcs`,
  `traffic.clear`, `network.registration_urc`, `network.lock_get`,
  `network.lock_apply`, `network.c5goption`, `network.c5goption_set`,
  `cell.neighbors`, `cell.scan_start`, `cell.scan_abort`, `beam.ssb`,
  `sim.slot_set`, `sim.hotplug_set`,
  `sim.pin_status`, `sim.pin_apply`, `system.nic_rate_set`,
  `system.power_control_set`, `system.factory_reset`, `modem.reset`,
  `modem.imei_set`, `network.radio_set`, `network.syscfg_set`,
  `system.thermal_set`, `system.fota_start`, `system.fota_abort`,
  `modem.nr_capability_set`) — an explicit user action that
  performs a live read; it may fail with a modem error, which the UI surfaces.
  It must never fail with a parameter/internal error (asserted by the registry
  test for every registered route).

| Route | Returns (domain JSON) |
| ----- | --------------------- |
| `signal.get` / `signal.cached` | `{sysmode, rssi, rsrp, rsrq, sinr, rscp, ecio}` |
| `network.get` / `network.cached` | `{operator, sysmode, sysmode_detail}` |
| `network.ims` | `{enabled?, registered?}` — on-demand `AT+CIREG?`; the wireless page's "IMS registration" row reads `registered` (1 = registered). A modem that rejects the query answers an empty object, which the page renders as no value |
| `registration.get` | `{state, tac, ci, act, nssai, mcc, mnc, lac}` |
| `qos.get` / `qos.cached` | `{active_cid, ambr_down_kbps, ambr_up_kbps, ambr_apn, qci}` |
| `network.pdp` | `{addresses: [{cid, address, family}]}` — on-demand `AT+CGPADDR` for the diagnostics panel |
| `network.dhcp` | `{ipv4: {address, netmask, gateway, dhcp_server, primary_dns, secondary_dns}, ipv6: {…}, ipv6_capability}` — on-demand `AT^DHCP?`/`AT^DHCPV6?`/`AT^IPV6CAP?` (the hex little-endian decode happens here, not in the page); partial answers keep the fields that were read |
| `network.registration_urc` | `{enabled: true}` — idempotent write `AT+CGREG=2`; the detailed PS registration report the Info page used to enable with raw AT |
| `network.lock_get` | `{lock_type, mobility, items: [{band, arfcn, pci, scs}]}` — `rat=lte\|nr`, on-demand `AT^LTEFREQLOCK?`/`AT^NRFREQLOCK?`; the `mobility,num` row, the per-carrier rows and the hex PCI are decoded here |
| `network.lock_apply` | `{cycled_radio, results: [{rat, applied, error?, code?}]}` — takes `{rat, lock_type, mobility, items}` or `{locks: [ … ]}` for both RATs in one radio cycle; the grouped-CSV write is assembled here, and each RAT's failure is reported separately |
| `network.c5goption` | `{nr_sa_support_flag, nr_dc_mode, gc_access_mode}` — on-demand `AT^C5GOPTION?` |
| `network.c5goption_set` | `{applied: true, cycled_radio}` — write with the radio cycled around it |
| `network.radio` | `{airplane: bool, cfun}` — display route over `+CFUN?`; an empty object when the modem did not answer, so the switch keeps its position |
| `network.radio_set` | `{applied: true, airplane}` — `{airplane: bool}` → `AT+CFUN=0\|1`; the network/registration snapshots are invalidated |
| `network.syscfg` | `{acqorder, band, roam, srvdomain, lteband}` — display route over `^SYSCFGEX?`; the reply's quoted-or-bare order field and the two trailing reserves are handled here, an unanswered read is an empty object |
| `network.syscfg_set` | `{applied: true}` — `{acqorder, band, roam, srvdomain, lteband}` → the seven-argument `AT^SYSCFGEX` write; the known acqorder list, the hex band masks and the roam/service-domain ranges are validated here (the CLI's `set-radio-policy` rules) |
| `network.autodial` | `{enable, dialMode, protocol, apn, username, password, authType}` — `^SETAUTODIAL?`; when autodial is off the firmware omits the mode and the module fills it from `^NDISSTATQRY?` (`dialMode: 1` = the modem dials itself), and an unread state answers `{}` so the read-only dial page keeps what it shows |
| `network.usb_mode` | `{mode}` — `^SETMODE?` |
| `network.interface_cfg` | `{mode, postRoute, dmz: {enabled, host}}` — one `^TDCFG?` read answers both of the dial page's cards (it used to issue the command twice); `Dmz: not cfg` is `enabled: false` |
| `network.pdp_contexts` | `{contexts: [{cid, type, apn, pdp_addr, active}]}` — `+CGDCONT?` joined with `+CGACT?` by cid, cids 1–20 only (0 is the modem's own context) |
| `network.schedule_get` | `{enabled, check_interval, timeout, unlock_lte, unlock_nr, toggle_airplane, night: {enabled, start, end, lte, nr}, day: {enabled, lte, nr}, status: {current_mode, next_switch, switch_count, applied}}` — the day/night band-lock config from UCI (`schedule_*`), typed and nested, plus the applier's live status. No modem access, so it answers on a dead modem too |
| `network.schedule_set` | `{applied: true}` — validates (HH:MM window, interval ≥10 s, timeout ≥30 s, lock type 0–3) and writes `uci set at-webserver.config.schedule_*` + `commit`; the applier re-reads every 15 s. `enabled` (the LuCI master switch) and `status` are read-only here, so no caller can lock itself out of the feature |
| `cell.neighbors` | `{cells: [{type, arfcn, pci, rsrp, rsrq, sinr, rxlev, band}]}` — on-demand `AT^MONNC`; hex PCI, the 1/8-unit NR scaling and the ARFCN→band table live here |
| `cell.scan_start` | `{started: true}` — `{rat?, plmn?, freq?, pci?, band?, scs?}`; validates the manual's constraints (band↔freq exclusivity, PCI only for LTE/NR, SCS required with an NR freq/PCI, band 1–512), builds `AT^CELLSCAN[=…]` including the band **bitmap** (`1 << (band-1)` in hex, nibble-built because n78 needs bit 77) and submits the exclusive task; rejects with `BUSY` while a scan runs |
| `cell.scan_result` | `{running, state, cells, count, raw?, error?}` — the last scan, from the task registry plus the `scan` cache (TTL 30 min); `raw` is the modem's `^CELLSCAN` reply text verbatim, which is what the LuCI modal and `mt5700m-at cellscan` print. Never touches the modem, so a page reload after a finished scan still renders it |
| `cell.scan_state` | `{running}` — task introspection only, never touches the modem, so a page reload finds a scan that outlived its mount |
| `cell.scan_abort` | `{aborted}` — cancels the scan task (the arbiter injects the firmware's abort token on the wire); `{aborted: false}` when none ran, so a cancel racing the scan's own completion is not an error |
| push `cellscan` | `{state: "done"\|"aborted"\|"error", cells: [{rat, ratName, plmn, freq, pci, band, lac, cid, rxlev, bsic, psc, scs, rsrp, rsrq, sinr, raw}], count, error?}` — published once per scan on the `scan` topic; the line layout (`^CELLSCAN:`, hex band/lac/cid, 1/2-dB RSRQ/SINR, 1/8-dB LTE SINR, the 15-vs-14 field quirk) is decoded in `modules/cell/scan.rs`, so no frontend parses it |
| `beam.ssb` | `{servingCell: {arfcn, cid, pci, rsrp, sinr, ta, ssbs: [{ssbId, rsrp}]}, neighborCells: [{pci, arfcn, rsrp, sinr, ssbs}]}` — on-demand `AT^NRSSBID?`; the fixed offsets and the "not measured" slots (255/32767) are handled here |
| `ca.get` / `ca.cached` | `{carriers: [{radio, band, source, dl_arfcn, ul_arfcn, dl_frequency_mhz, ul_frequency_mhz, dl_bandwidth_mhz, ul_bandwidth_mhz}], secondary: [{radio: "NR", arfcn, pci, rsrp?, rsrq?, sinr?, measType} \| {radio: "LTE", index, pci, band, rssi?, rsrp?, rsrq?, ulArfcn?, dlArfcn?, ulFreq?, dlFreq?, ulBandwidth?, dlBandwidth?}], carrier_count, ca_active, dc_active, nr_carrier_count, lte_carrier_count, lte_secondary_count, secondary_connection_count, ca_mode, ca_dl_bandwidth, ca_ul_bandwidth}` (`ca.get?refresh=1` forces a live read) — `secondary` is `^CASCELLINFO?` + `^MONSSC`: the per-carrier signal the `^HFREQINFO?` list does not carry (hex PCI, the manual's invalid values -1256/-348/-188, the ±8 descale heuristic and `<MEASTYPE>` all decoded here), so the info page merges the two by downlink ARFCN instead of parsing `^MONSSC` itself |
| `cell.get` / `cell.cached` | `{band, channel, dlBandwidth, arfcn, sysmode, mcc, mnc, cid, pci, lac, operator, raw}` |
| `sim.get` / `sim.cached` | `{status, iccid, imsi, slot, hotplug}` (+ `number` once read) — the periodic snapshot also carries the active slot and the hot-plug switch |
| `sim.number` | `{…, number}` — reads `+CNUM` on demand |
| `sim.slot` | `{slot, hotplug}` — display route over the `sim` topic (0 = external, 1 = internal) |
| `sim.slot_set` | `{switched: true, slot}` — `{slot: 0\|1}`; the vendor sequence (`^HVSST` deactivate/activate around `^SCICHG`, radio off/on) is performed here, once |
| `sim.hotplug_set` | `{applied: true, hotplug}` — `{hotplug: bool}` → `^TDSIMHP` |
| `sim.pin_status` | `{code, lock, blocked, needsNewPin, card: {status, dead, present}, pinEnabled}` — `+CPIN?` with the CME-error branch (10 → `ABSENT`, 11/12/17/18 → the matching lock), `^SIMSQ?` for the dead/present refinement and `+CLCK="SC",2` when the card is ready |
| `sim.pin_apply` | `{applied: true}` — `{operation: verify\|unblock\|enable\|disable\|change, pin, newPin?, pin2?}`; the operation picks CPIN/CLCK/CPWD, and the 4–8 digit rules are validated here |
| `modem.get` / `modem.cached` | `{manufacturer, model, revision, imei}` |
| `modem.txpower` | `{total, pusch, pucch, srs, prach}` |
| `modem.nr_txpower` | `{carriers: [{pusch, pucch, srs, prach, freq}]}` |
| `modem.endc` | `{available, plmnAvailable, restricted, established}` |
| `modem.mcs` | `{downlink: {rat, carriers: [{index, group, rat, mcs_table_index, code0, code1}], avg_mcs}, uplink: {…}}` — on-demand `AT^MCS=1` / `AT^MCS=0`; `group`/`rat` are the reply's own per-line grouping (LuCI prints one block per `^MCS` line, the WebUI ignores them and pairs carriers with `^HFREQINFO` by position), `rat` at the direction level is the merged view (NR wins); the frontends map `code0` to modulation/level labels |
| `modem.reset` | `{rebooting: true}` — `AT^RESET`; the snapshot is dropped so the next read is post-restart |
| `modem.imei_set` | `{applied: true, imei}` — `{imei: "15 digits"}` → `^PHYNUM=IMEI,<imei>`; the digit rule is validated here |
| `modem.nr_capability` | `{ca, vonr, dss: {rateMatchingLTE, additionalDMRS}}` — display route over `^NRRCCAPQRY=3/2/5`; each reply echoes its kind (the parser matches on it) and an ability that did not answer stays absent, including half a DSS pair |
| `modem.nr_capability_set` | `{applied: true, wrote: [kind…]}` — `{ca?, vonr?, dss?: {rateMatchingLTE, additionalDMRS}}`, at least one; each ability becomes its own `^NRRCCAPCFG` write, with the VoNR 0–3 and DSS 0/1 ranges validated here |
| `traffic.get` / `traffic.cached` | PDCP field map (`id`, `pduSessionId`, …, `dlDiscardCnt`) |
| `traffic.netrate` | `{available, device, rx_bytes, tx_bytes, timestamp, source, traffic}` |
| `traffic.clear` | `{cleared: true}` — on-demand write `AT^DSFLOWCLR`; the scheduler's action-invalidation drops the stale counters |
| `system.temperature` / `system.temperature.cached` | 12 sensor fields + `average` |
| `system.device_control` | `{nic_rate: 1\|2, power_control: bool}` — `^TDPCIELANCFG?` and `^TDPMCFG?`; a switch that did not answer is omitted, and the page keeps the value it shows |
| `system.nic_rate_set` | `{applied: true, nic_rate}` — `{rate: 1\|2}` → `^TDPCIELANCFG=<rate>` (takes effect after a reboot) |
| `system.power_control_set` | `{applied: true, power_control}` — `{enabled: bool}` → `^TDPMCFG=<0\|1>` |
| `system.factory_reset` | `{restored: true}` — `AT&F` (AT defaults; the modem is not restarted) |
| `system.service_mode` | `{mode: "serial"\|"network"}` — how the daemon reaches the modem, recorded once at startup (`core::modem`); replaces the pages' `AT+CONNECT?` probe |
| `system.thermal` | `{enabled, caMimoSwitch, interval, logSwitch: {consoleLog, fileLog}, thresholds: [...], currentLevel}` — `^THERMAUTOFUN?` / `^THERMLDLOGSW?` / `^THERMLDAUTOPARA?` / `^THERMLDAUTOSTATUS?` (the level is the 6th status field, decoded here); a query that did not answer leaves its fields absent |
| `system.thermal_set` | `{applied: true}` — `{enabled, caMimoSwitch?, interval}` → `^THERMAUTOFUN=<on>,<caMimo>,<interval>`; the interval range is validated here |
| `system.fota` | `{running, phase: "idle"\|"running"\|"done"\|"error", step, progress, state, stateName, total, received, error?}` — the upgrade flow's state from the **task registry plus the published snapshot**, never an AT access, so a page reload (or its 1 s poll) cannot queue behind the download it is reporting on |
| `system.fota_start` | `{started: true}` — `{url}`; the address rules (`http://` only, empty rejected, trailing slash added, quote guard) are validated here and the module submits its `fota.system` task, which runs the whole flow: `ATE0`, `^FOTAMODE=0,1,0,1`, `^FOTAOEMDL="…"`, then the state machine (`^FOTASTATE?` every 1 s, `^FOTADLQ` progress on 30, resume on 31 at most once per 5 s, `^FWUP` on 40). Rejects with `BUSY` while a flow runs |
| `system.fota_abort` | `{aborted}` — cancels the flow task; `{aborted: false}` when none ran. The WebUI's page has no cancel button (UI unchanged), so this is the terminal/API escape hatch |
| push `fota.progress` | `{running, phase, step, progress, state, stateName, total, received, error?}` — published on the `fota` topic on every state change (immediate delivery), the same object `system.fota` answers with |
| `sms.status` | `{enabled, imsOn?, center?, storage?}` — page-load snapshot: `+CMGF?` (the read that decides 短信是否开启), `^IMSSWITCH?`, `+CSCA?` (only when IMS is on, as the page always did) and `+CPMS?`; a missing/unread field is omitted and `enabled` is always present |
| `sms.storage` | `{read, write, receive, storages: [name…]}` — `+CPMS?` decoded (`name`, `used`, `total` per plane, distinct names for the clear-all loop) |
| `sms.list` | `{messages: [{index, content, number, time, type, isConcatenated?, concatenatedRef/Seq/Total?}]}` — `+CMGF=0` when needed, then `+CMGL=4`; every PDU is decoded and multipart parts are merged in sequence order, so no frontend reassembles a PDU |
| `sms.send` | `{sent: true, parts}` — `{number, text}`; the destination is normalised (11-digit local numbers get country code 86) and encoded here (GSM 7-bit / UCS-2, multipart split, service centre from `+CSCA?` — `00` when the modem reports none), then sent as one `AT+CMGS` transaction per part on the arbiter thread (`AT+CMGS=<len>` counts the TPDU octets only); a multipart failure reads `第 i/n 条发送失败：<cause>` |
| `sms.delete` | `{deleted: true}` — `{index}` → `+CMGD=<index>` |
| `sms.clear_all` | `{cleared: [storage…]}` — reads `+CPMS?` itself, then per plane `+CPMS="X","X","X"` + `+CMGD=1,4` with the firmware's settle times (the settings page's 清空所有短信) |
| `sms.storage_set` | `{applied: true}` — `{read, write?, receive?}` (defaults to `read`) → `+CPMS=…`; storage names are validated against `SM`/`ME` here |
| `sms.center_set` | `{applied: true}` — `{number}` → `+CSCA="<number>"`; empty/illegal characters rejected here |
| `sms.ims_set` | `{applied: true}` — `{enabled}` runs the module's five-step IMS sequence (`+CFUN=0` → IMS PDP profile → `+CEUS` → `^IMSSWITCH` → `+CFUN=1`) with its settle times, one implementation shared by both frontends and the CLI |
| `sms.analyze` | `{encoding: "7bit"\|"UCS2", chars, parts}` — `{text}`; the compose hint's part count comes from the same codec `sms.send` uses, so the promise and the send cannot disagree |
| `sms.ussd_send` | `{sent: true, reply?}` — `{code}`; the code is validated and packed here (GSM 7-bit, the manual's own example `*133#` → `AAD86C3602`, sent as `AT+CUSD=1,"…",15`), a rejected code answers the panel's copy verbatim, and a firmware that returns the answer inline gets it decoded into `reply` (`{m, mText, text, needsReply}`) |
| `sms.ussd_cancel` | `{cancelled: true}` — `AT+CUSD=2`, releasing the session |
| push `network.reject` | `{plmn, domain, domainText, cause, causeText, rat, ratText, rejectType, rejectTypeText, originalCause, lac, rac, cellId, esmCause?, raw, at}` — a `^REJINFO` line (手册 13.14) decoded by `modules/network/reject.rs`: the cause table, the USIM range 65537–65543 and the domain/rat/type labels live here, not in a page (the `network` topic) |
| push `qos.ambr` | `{ambr_down_kbps, ambr_up_kbps, ambr_apn?}` — an unsolicited `^DSAMBR` line (手册 5.33) decoded by `modules/qos` with the same field names `qos.get` uses, so the info page updates APN/AMBR without polling (the `qos` topic) |
| push `sim.changed` | the raw line (`+CPIN:` / `^SIMSQ:` / `^SIMST`) — a nudge on the `sim` topic: the card handler re-reads `sim.pin_status` instead of pattern-matching URC text in the browser |
| push `sms.ussd` | `{m, mText, text, needsReply}` — the `+CUSD:` line the network sends out-of-band, decoded by the same codec (the `sms` topic). The raw line still goes out as `raw_data`, but no page parses it |

Field names and types are exactly what the cache published before the refactor,
so existing consumers (LuCI topics, WebUI `stateCache`, `mt5700m-at cached`)
keep working unchanged. Absent fields are omitted rather than sent as `null`,
matching the previous writers.

## 2. Transports and envelopes

| Transport | Call shape | Success | Failure |
| --------- | ---------- | ------- | ------- |
| WebSocket (WebUI) | `{"method":"api","params":{"path":"signal.get"}}` | `{success: true, data: {…}}` | `{success: false, error, code, retryable}` |
| WS command line (WebUI service layer) | `api.signal.get` / `api.ca.get {"refresh":true}` | `{success: true, data: {…}}` | as above |
| WS legacy | `{"method":"at","params":{"cmd":"AT+CSQ"}}` | `{success: true, data: "<text>"}` | as above |
| Control socket (CLI) | `{"cmd":"api","method":"signal.get","params":{}}\n` | `{ok: true, result: {…}}` | `{ok: false, error, code}` |
| TCP RPC (ucode/LuCI) | `{"id":1,"method":"api","params":{"path":"signal.get"}}` | `{success: true, data: {…}}` | as above |
| TCP RPC `at` (LuCI `mt5700.at`) | `{"cmd":"api.beam.ssb"}` (optional trailing JSON = params) | `{success: true, data: {…}}` | as above |
| `mt5700m-at` CLI | `mt5700m-at <verb>` | legacy text on stdout | exit code + stderr (`124` on timeout) |

`error` in the envelopes is `BackendError::detail()`: for a rejected parameter
it is the bare rejection text ("频段与频点不能同时指定") — the exact copy the
pages showed when they validated locally — and for everything else it is the
same `message()` the logs and the CLI print. `code` is always `Error::code()`.

A route is reachable through every transport, and the `api.` prefix is
normalised inside `registry::dispatch`, so `api.signal.get` (WS/LuCI command
line, `split_api_command` carries the optional trailing JSON as `params`) and
`signal.get` (control socket, JSON-RPC, CLI) hit the same handler. Frontends
call routes; only the deliberate raw-AT consoles (`WebUI /at` terminal, LuCI
`terminal.js`) still send AT.

LuCI reaches the registry through `api.js`'s `route(name, params)` helper —
`mt5700.at` with `cmd = api.<route> [json]` — so a page asks for a domain model
(`api.route('beam.ssb')`) instead of a labelled text frame it would have to
slice. The render-only helpers stay on the CLI verbs; the pages migrate one
section at a time (see `migration.md` item 9).

Legacy verbs are preserved byte for byte: control socket `send|cached|scan`
(+ `api`; the old `sms` verb is gone — `sms.send` is the one send path),
TCP RPC `at|cached|events|scan|ping` (+ `api`), WS events
`{type, data, timestamp}` with topic names `signal[.updated]`,
`network.updated`, `cell.updated`, `temperature.updated`, `traffic.updated`,
`netrate.updated`, `registration.updated`, `endc.updated`, `txpower.updated`,
`nr_txpower.updated`, `sim.updated`, `modem.info` and the `^(usb|modem|task|scan|beam|sms)\.`
family. Auth failures keep the exact strings `Authentication failed`,
`Authentication timeout`, `Invalid authentication`.

## 3. Timeout budget (unchanged)

```text
frontend hard timeout 30 s
  > ucode write class 25 s / read class 12 s
    > daemon queued 10 s + exec 8 s + 2 s slack
CLI capture hard cap 25 s (exit 124)
```

## 4. Adding a capability

1. Add the route to the owning module's `api.rs` (`Route { name, handler }`).
2. Register it in `api/registry.rs::routes()`.
3. Both frontends (and the CLI) can call it immediately — no transport work, no
   duplicated parsing, no new cache key unless the module publishes one.

Any route must answer on a cold cache with no modem attached: registry tests
assert exactly that for every registered route.
