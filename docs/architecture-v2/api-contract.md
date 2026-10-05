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
* **on-demand** (`sim.number`, `network.pdp`) — an explicit user action that
  performs a live read; it may fail with a modem error, which the UI surfaces.
  It must never fail with a parameter/internal error (asserted by the registry
  test for every registered route).

| Route | Returns (domain JSON) |
| ----- | --------------------- |
| `signal.get` / `signal.cached` | `{sysmode, rssi, rsrp, rsrq, sinr, rscp, ecio}` |
| `network.get` / `network.cached` | `{operator, sysmode, sysmode_detail}` |
| `registration.get` | `{state, tac, ci, act, nssai, mcc, mnc, lac}` |
| `network.pdp` | `{addresses: [{cid, address, family}]}` — on-demand `AT+CGPADDR` for the diagnostics panel |
| `ca.get` / `ca.cached` | `{carriers: [{radio, band, source, dl_arfcn, ul_arfcn, dl_frequency_mhz, ul_frequency_mhz, dl_bandwidth_mhz, ul_bandwidth_mhz}], carrier_count, ca_active, dc_active, nr_carrier_count, lte_carrier_count, lte_secondary_count, secondary_connection_count, ca_mode, ca_dl_bandwidth, ca_ul_bandwidth}` (`ca.get?refresh=1` forces a live read) |
| `cell.get` / `cell.cached` | `{band, channel, dlBandwidth, arfcn, sysmode, mcc, mnc, cid, pci, lac, operator, raw}` |
| `sim.get` / `sim.cached` | `{status, iccid, imsi}` (+ `number` once read) |
| `sim.number` | `{…, number}` — reads `+CNUM` on demand |
| `modem.get` / `modem.cached` | `{manufacturer, model, revision, imei}` |
| `modem.txpower` | `{total, pusch, pucch, srs, prach}` |
| `modem.nr_txpower` | `{carriers: [{pusch, pucch, srs, prach, freq}]}` |
| `modem.endc` | `{available, plmnAvailable, restricted, established}` |
| `traffic.get` / `traffic.cached` | PDCP field map (`id`, `pduSessionId`, …, `dlDiscardCnt`) |
| `traffic.netrate` | `{available, device, rx_bytes, tx_bytes, timestamp, source, traffic}` |
| `system.temperature` / `system.temperature.cached` | 12 sensor fields + `average` |

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
| `mt5700m-at` CLI | `mt5700m-at <verb>` | legacy text on stdout | exit code + stderr (`124` on timeout) |

A route is reachable through every transport, and the `api.` prefix is
normalised inside `registry::dispatch`, so `api.signal.get` (WS/LuCI command
line, `split_api_command` carries the optional trailing JSON as `params`) and
`signal.get` (control socket, JSON-RPC, CLI) hit the same handler. Frontends
call routes; only the deliberate raw-AT consoles (`WebUI /at` terminal, LuCI
`terminal.js`) still send AT.

Legacy verbs are preserved byte for byte: control socket `send|sms|cached|scan`
(+ `api`), TCP RPC `at|cached|events|scan|ping` (+ `api`), WS events
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
