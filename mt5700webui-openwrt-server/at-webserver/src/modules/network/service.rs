//! Network service: refresh policy for the `network` and `registration`
//! topics, plus registration of the periodic jobs.
//!
//! Two jobs, matching the cadences the frontends were tuned against:
//!   * `network.registration`, 20 s — C5GREG first (tac/ci/AcT/NSSAI), falling
//!     back to CEREG then CREG; the topic is always written (Null when nothing
//!     parsed) so consumers keep a stable TTL;
//!   * `network.refresh`, 30 s — operator (`+COPS?`, 60 s failure backoff) and
//!     system mode (`^SYSINFOEX`, 300 s failure backoff), both non-fatal.

use crate::core::channel::AtChannel;
use crate::core::error::BackendError;
use crate::core::json::Value;
use crate::core::task::Priority;
use crate::modules::network::commands::{self, COPS, SYSCFGEX_QUERY, SYSINFOEX};
use crate::modules::network::parser;
use crate::modules::network::state::{
    AutodialState, C5gOptionState, InterfaceCfgState, LockKind, LockState, NetworkState, PdpContext,
    RegistrationState, SysCfgState, UsbModeState,
};
use crate::scheduler::channel::run_in_task;
use crate::scheduler::jobs::TaskManager;
use crate::state::bus::{
    EVENT_NETWORK_UPDATED, EVENT_REGISTRATION_UPDATED, TOPIC_NETWORK, TOPIC_REGISTRATION,
};
use crate::state::cache::StateCache;
use crate::state::refresh::RefreshCtx;
use std::sync::Arc;
use std::thread::sleep;
use std::time::Duration;

const REG_AT_TIMEOUT: Duration = Duration::from_secs(8);
const REG_QUEUED_TIMEOUT: Duration = Duration::from_secs(6);
const REG_PERIOD: Duration = Duration::from_secs(20);
const PERIOD: Duration = Duration::from_secs(30);
/// Backoff keys are part of the observable cache (`snapshot()` exposes them to
/// the diagnostics page), so they keep their historical names.
const COPS_BACKOFF_KEY: &str = "network_cops";
const SYSINFO_BACKOFF_KEY: &str = "network_sysinfo";

/// Refresh the registration topic (`+CxxREG`).
pub fn refresh_registration(ctx: &RefreshCtx) -> RegistrationState {
    let mut reg = RegistrationState::default();
    for cmd in commands::REG_QUERIES {
        if let Ok(text) = ctx.read(cmd, REG_AT_TIMEOUT, REG_QUEUED_TIMEOUT, Priority::Normal) {
            let t = text.trim();
            if !t.is_empty() && commands::has_registration_line(t) {
                reg = parser::parse_registration(t);
                break;
            }
        }
    }
    ctx.store(
        TOPIC_REGISTRATION,
        EVENT_REGISTRATION_UPDATED,
        &reg.to_json(),
    );
    reg
}

/// Refresh operator + access technology for the `network` topic.
pub fn refresh(ctx: &RefreshCtx) -> Result<NetworkState, BackendError> {
    let mut st = NetworkState {
        registration: cached_registration(ctx.cache).unwrap_or_default(),
        ..Default::default()
    };

    // COPS is slow on the MT5700M (8 s+), so it is backoff-protected instead of
    // retried: during the backoff window the previous value is kept.
    if let Some(cops) = ctx.slow(
        COPS_BACKOFF_KEY,
        COPS,
        Duration::from_secs(8),
        Duration::from_secs(6),
        Duration::from_secs(60),
    ) {
        st.operator = parser::parse_cops_operator(&cops);
        st.sysmode = parser::parse_cops_rat(&cops);
    }
    if let Some(sysinfo) = ctx.slow(
        SYSINFO_BACKOFF_KEY,
        SYSINFOEX,
        Duration::from_secs(6),
        Duration::from_secs(5),
        Duration::from_secs(300),
    ) {
        st.sysmode_detail = parser::parse_sysinfo_mode(&sysinfo);
    }

    // Publish even when empty: the previous value survives through the cache
    // TTL, and a genuine "no service" answer must be visible to the UI.
    ctx.store(TOPIC_NETWORK, EVENT_NETWORK_UPDATED, &st.to_json());
    Ok(st)
}

/// Read the access-technology configuration (`^SYSCFGEX?`).
///
/// The reply layout (quoted or bare `acqorder`, the two trailing reserves) is
/// decoded by `parser::parse_syscfgex`; a reply without the expected line is an
/// `AtRejected`, which the page treats as "keep what I show".
pub fn read_syscfg(ctx: &RefreshCtx) -> Result<SysCfgState, BackendError> {
    let text = ctx.read(
        SYSCFGEX_QUERY,
        Duration::from_secs(8),
        Duration::from_secs(6),
        Priority::Normal,
    )?;
    parser::parse_syscfgex(&text).ok_or_else(|| {
        BackendError::AtRejected(format!("^SYSCFGEX? answered an unknown shape: {}", text.trim()))
    })
}

/// Write the access-technology configuration.
///
/// Validation mirrors what the modem accepts (the CLI's `set-radio-policy`
/// rules): a known `acqorder`, non-empty hex band masks, roam 0/1 and
/// service-domain 0..=2 (the page offers 0 as "voice only").
pub fn apply_syscfg(
    ctx: &RefreshCtx,
    acqorder: &str,
    band: &str,
    roam: i64,
    srvdomain: i64,
    lteband: &str,
) -> Result<(), BackendError> {
    if !commands::valid_acq_order(acqorder) {
        return Err(BackendError::InvalidParameter(format!(
            "网络制式优先级无效: {}",
            acqorder
        )));
    }
    for (label, value) in [("band", band), ("lteband", lteband)] {
        if !commands::is_hex_mask(value) {
            return Err(BackendError::InvalidParameter(format!(
                "{} 必须是十六进制掩码，收到 {}",
                label, value
            )));
        }
    }
    if !(0..=1).contains(&roam) {
        return Err(BackendError::InvalidParameter(format!(
            "漫游设置只能是 0 或 1，收到 {}",
            roam
        )));
    }
    if !(0..=2).contains(&srvdomain) {
        return Err(BackendError::InvalidParameter(format!(
            "服务域只能是 0-2，收到 {}",
            srvdomain
        )));
    }
    ctx.action(&commands::syscfgex(
        acqorder,
        band,
        roam as u8,
        srvdomain as u8,
        lteband,
    ))?;
    ctx.cache.invalidate_many(&[TOPIC_NETWORK, TOPIC_REGISTRATION]);
    Ok(())
}

/// Cache-first read for the API: the last published domain value.
pub fn cached(cache: &Arc<StateCache>) -> Option<NetworkState> {
    let (value, _) = cache.get(TOPIC_NETWORK);
    match value {
        Some(Value::Obj(m)) => {
            let mut st = NetworkState::from_json(&m);
            st.registration = cached_registration(cache).unwrap_or_default();
            Some(st)
        }
        _ => None,
    }
}

/// The registration topic on its own (the cell card reads TAC/CI from it).
pub fn cached_registration(cache: &Arc<StateCache>) -> Option<RegistrationState> {
    match cache.get(TOPIC_REGISTRATION) {
        (Some(Value::Obj(m)), _) => Some(RegistrationState {
            state: m.get("state").and_then(|v| v.as_i64()).unwrap_or(0) as u8,
            tac: str_field(&m, "tac"),
            ci: str_field(&m, "ci"),
            act: str_field(&m, "act"),
            nssai: str_field(&m, "nssai"),
            mcc: str_field(&m, "mcc"),
            mnc: str_field(&m, "mnc"),
            lac: str_field(&m, "lac"),
        }),
        _ => None,
    }
}

fn str_field(m: &std::collections::BTreeMap<String, Value>, key: &str) -> Option<String> {
    m.get(key).and_then(|v| v.as_str()).map(|s| s.to_string())
}

// ------------------------------------------------------------ frequency lock
//
// The lock apply sequence is one implementation for every caller: the CLI
// (`mt5700m-at lock`), the WebUI route (`network.lock_apply`) and the day/night
// scheduler all go through it, so the airplane-mode dance, the per-command
// independence and the verification cannot drift apart. The AT strings
// themselves are built by `commands::lte_lock_command`/`nr_lock_command`.

/// One lock write to perform: the RAT it belongs to and the exact command.
pub struct LockApply {
    pub kind: LockKind,
    pub command: String,
}

/// What an apply sequence did, for callers that need to know whether the radio
/// was cycled (the CLI only polls for the new lock when the firmware had to be
/// restarted for it).
pub struct LockApplyReport {
    pub outcomes: Vec<LockApplyOutcome>,
    /// True when `+CFUN=0` … `+CFUN=1` was performed around the writes.
    pub cycled_radio: bool,
}

/// Per-RAT result of an apply sequence, in the order the writes were issued.
pub struct LockApplyOutcome {
    pub kind: LockKind,
    /// The raw modem answer, for callers that print it (the CLI).
    pub text: String,
    /// First error seen for this RAT, if any.
    pub error: Option<BackendError>,
}

impl LockApplyOutcome {
    pub fn ok(&self) -> bool {
        self.error.is_none()
    }
}

/// Read the current lock of one RAT (`^LTEFREQLOCK?` / `^NRFREQLOCK?`).
pub fn read_lock(channel: &dyn AtChannel, kind: LockKind) -> Result<LockState, BackendError> {
    let text = channel.query_prio(
        commands::lock_query(kind),
        Duration::from_secs(8),
        Duration::from_secs(10),
        Priority::Normal,
    )?;
    Ok(parser::parse_freq_lock(&text, kind).unwrap_or_default())
}

/// Apply one or more lock writes, cycling the radio exactly the way the pages
/// always have:
///
/// 1. read `+CFUN?` — if the radio is on (and the caller wants a cycle), take
///    it down first, because the firmware only exposes a lock change after a
///    function-level cycle;
/// 2. issue each RAT's write independently: a modem that rejects the NR lock
///    must not stop the LTE one (and vice versa), so the first error of each
///    RAT is kept and returned with its result;
/// 3. bring the radio back up and give the modem 2 s to re-register.
///
/// Errors that mean "the whole sequence cannot start" (CFUN refused) are
/// returned as `Err`; per-lock failures come back inside the outcomes.
pub fn apply_lock(
    channel: &dyn AtChannel,
    applies: &[LockApply],
    cycle_radio: bool,
) -> Result<LockApplyReport, BackendError> {
    let radio_on = radio_cycle_begin(channel, cycle_radio)?;
    let mut out: Vec<LockApplyOutcome> = Vec::new();
    for (i, a) in applies.iter().enumerate() {
        let result = channel.action(&a.command);
        let (text, error) = match result {
            Ok(t) => (t, None),
            Err(e) => (String::new(), Some(e)),
        };
        out.push(LockApplyOutcome {
            kind: a.kind,
            text,
            error,
        });
        if i + 1 < applies.len() {
            sleep(Duration::from_secs(1));
        }
    }
    if radio_on {
        radio_cycle_end(channel)?;
    }
    Ok(LockApplyReport {
        outcomes: out,
        cycled_radio: radio_on,
    })
}

/// Take the radio down for a settings write when it is currently on.
///
/// Returns `true` when the caller must call [`radio_cycle_end`] afterwards. The
/// vendor firmware only re-reads several settings (frequency lock, 5G access
/// mode) after a function-level cycle, and an airplane-mode session that was
/// already offline is left offline on purpose — hence the `+CFUN?` probe.
fn radio_cycle_begin(channel: &dyn AtChannel, cycle: bool) -> Result<bool, BackendError> {
    let previous = channel
        .query(commands::CFUN_QUERY)
        .ok()
        .and_then(|t| parser::parse_cfun(&t));
    let radio_on = cycle && previous != Some(0);
    if radio_on {
        channel.action(&commands::cfun(0))?;
        sleep(Duration::from_secs(1));
    }
    Ok(radio_on)
}

/// Bring the radio back up and give the modem a moment to re-register.
fn radio_cycle_end(channel: &dyn AtChannel) -> Result<(), BackendError> {
    channel.action(&commands::cfun(1))?;
    sleep(Duration::from_secs(2));
    Ok(())
}

/// Write one setting that only takes effect after a function-level cycle.
///
/// `Ok((raw, cycled))`: the modem's answer to the write and whether the radio
/// was cycled around it. The page's 5G-access-mode switch and the CLI's future
/// verbs both go through here — the same sequence as the lock apply.
pub fn write_with_radio_cycle(
    channel: &dyn AtChannel,
    command: &str,
) -> Result<(String, bool), BackendError> {
    let cycled = radio_cycle_begin(channel, true)?;
    let result = channel.action(command)?;
    if cycled {
        radio_cycle_end(channel)?;
    }
    Ok((result, cycled))
}

/// Read the 5G access mode (`^C5GOPTION?`).
pub fn read_c5goption(channel: &dyn AtChannel) -> Result<C5gOptionState, BackendError> {
    let text = channel.query_prio(
        commands::C5GOPTION_QUERY,
        Duration::from_secs(8),
        Duration::from_secs(10),
        Priority::Normal,
    )?;
    Ok(parser::parse_c5goption(&text))
}

/// Outcome of a lock verification poll.
pub struct LockVerify {
    /// Raw answer of the last lock query (the CLI prints it verbatim).
    pub text: String,
    pub ok: bool,
    /// Lock type the modem reported last (`""` when nothing parseable came
    /// back), for the caller's error message.
    pub observed: String,
}

/// Poll the lock query until it reports `expected` (up to `attempts` tries,
/// 2 s apart) and return the last raw answer. The CLI prints it.
pub fn verify_lock(
    channel: &dyn AtChannel,
    kind: LockKind,
    expected: &str,
    attempts: u32,
) -> LockVerify {
    let mut text = String::new();
    let mut observed = String::new();
    for i in 0..attempts {
        if let Ok(answer) = channel.query(commands::lock_query(kind)) {
            text = answer;
            observed = parser::parse_freq_lock(&text, kind)
                .map(|st| st.lock_type.to_string())
                .unwrap_or_default();
            if observed == expected {
                return LockVerify {
                    text,
                    ok: true,
                    observed,
                };
            }
        }
        if i + 1 < attempts {
            sleep(Duration::from_secs(2));
        }
    }
    LockVerify {
        text,
        ok: false,
        observed,
    }
}

/// Register both periodic jobs on the shared task manager.
pub fn spawn(tasks: &TaskManager) {
    tasks.add_periodic(
        "network.registration",
        REG_PERIOD,
        Priority::Normal,
        Some(REG_PERIOD),
        Box::new(|ctx| {
            run_in_task(ctx, |r| {
                refresh_registration(r);
                Ok(Value::Null)
            })
        }),
    );
    tasks.add_periodic(
        "network.refresh",
        PERIOD,
        Priority::Normal,
        Some(PERIOD),
        Box::new(|ctx| run_in_task(ctx, |r| refresh(r).map(|st| st.to_json()))),
    );
}

// ------------------------------------------------------------ dial page reads
//
// All four are live reads on a page-load path, so they use the same budget as
// the other configuration queries (8 s AT / 6 s queue, Normal priority); the
// display routes turn a failure into an empty object, and the page keeps the
// value it is showing.

/// Built-in autodial configuration, plus the NDIS fallback the page used when
/// the reply omits the dial mode.
pub fn read_autodial(ctx: &RefreshCtx) -> Result<AutodialState, BackendError> {
    let text = ctx.read(
        commands::SETAUTODIAL_QUERY,
        Duration::from_secs(8),
        Duration::from_secs(6),
        Priority::Normal,
    )?;
    let mut st = parser::parse_autodial(&text);
    if st.is_empty() {
        return Err(BackendError::AtRejected(format!(
            "^SETAUTODIAL? answered an unknown shape: {}",
            text.trim()
        )));
    }
    // MT5700 关闭内置自动拨号时只回 ^SETAUTODIAL:0，没有数据接口字段；此时用
    // 正在工作的 NDIS 会话判断 USB 数据口，避免把主机的 USB 拨号显示成"转网口"。
    if st.dial_mode.is_none() {
        if let Ok(ndis) = ctx.read(
            commands::NDISSTATQRY,
            Duration::from_secs(6),
            Duration::from_secs(5),
            Priority::Normal,
        ) {
            st.ndis_active = commands::ndis_is_active(&ndis);
        }
    }
    Ok(st)
}

/// USB port mode (`^SETMODE?`).
pub fn read_usb_mode(ctx: &RefreshCtx) -> Result<UsbModeState, BackendError> {
    let text = ctx.read(
        commands::SETMODE_QUERY,
        Duration::from_secs(8),
        Duration::from_secs(6),
        Priority::Normal,
    )?;
    let mode = parser::parse_usb_mode(&text).ok_or_else(|| {
        BackendError::AtRejected(format!("^SETMODE? answered an unknown shape: {}", text.trim()))
    })?;
    Ok(UsbModeState { mode: Some(mode) })
}

/// Interface configuration (`^TDCFG?`) — one read for the page's two cards.
pub fn read_interface_cfg(ctx: &RefreshCtx) -> Result<InterfaceCfgState, BackendError> {
    let text = ctx.read(
        commands::TDCFG_QUERY,
        Duration::from_secs(8),
        Duration::from_secs(6),
        Priority::Normal,
    )?;
    let st = parser::parse_interface_cfg(&text);
    if st.mode.is_none() && st.post_route.is_none() && !st.dmz_enabled {
        return Err(BackendError::AtRejected(format!(
            "^TDCFG? answered an unknown shape: {}",
            text.trim()
        )));
    }
    Ok(st)
}

/// PDP context table (`+CGDCONT?` + `+CGACT?`).
pub fn read_pdp_contexts(ctx: &RefreshCtx) -> Result<Vec<PdpContext>, BackendError> {
    let defs = ctx.read(
        commands::CGDCONT_QUERY,
        Duration::from_secs(8),
        Duration::from_secs(6),
        Priority::Normal,
    )?;
    let actives = ctx
        .read(
            commands::CGACT_QUERY,
            Duration::from_secs(8),
            Duration::from_secs(6),
            Priority::Normal,
        )
        .unwrap_or_default();
    Ok(parser::parse_pdp_contexts(&defs, &actives))
}
