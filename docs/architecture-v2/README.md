# MT5700M architecture v2

One backend, two independent frontends:

```text
LuCI ──rpc/ucode──┐
                  ├──▶ Unified Rust backend (at-webserver) ──▶ AT scheduler ──▶ serial
WebUI ──WebSocket─┘
```

The backend owns the modem. Frontends own pixels: they render domain JSON,
send named API calls, and keep only UI-local state (active tab, dialog, form
contents, loading flags).

## Documents

| File | Contents |
| ---- | -------- |
| [architecture.md](architecture.md) | Layers, module tree, dependency rules, what may depend on what |
| [data-flow.md](data-flow.md) | The AT → Parser → Domain → Cache → API → UI pipeline, event bus, task lifecycle, single-AT-owner sequence |
| [api-contract.md](api-contract.md) | Route table, envelopes per transport, compatibility guarantees (CLI/ucode/WebSocket) |
| [module-guide.md](module-guide.md) | Recipe: add a feature as a module (State/Commands/Parser/Service/API) |
| [migration.md](migration.md) | What moved in this refactor, what is verified, what remains |

## The five rules

1. **One AT owner.** The daemon's `serial::manager` holds the tty (TIOCEXCL).
   Every other process reaches the modem through the control socket.
2. **One implementation per capability.** A parser, a cache, an API handler or
   a retry policy exists once, in the module that owns the domain.
3. **Modules talk through interfaces, not internals.** `AtChannel` for AT,
   `RefreshCtx` for cache+bus, `ApiCtx`/`Route` for the API, cached topics for
   cross-module data. No module touches the serial port, the arbiter, another
   module's private items or a frontend format.
4. **One state source.** The daemon's `StateCache` + `EventBus` is the only
   business state; frontends may not derive it from AT text.
5. **UI is frozen.** Layout, styles, copy, routes and interaction flows are
   unchanged by construction: the refactor moves *where data comes from*, never
   what is drawn.
