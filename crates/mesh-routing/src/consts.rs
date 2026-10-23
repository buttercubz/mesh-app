//! Routing, floor and pacing constants (DESIGN Appendix A, §5, §6.1). Times are µs,
//! rates bps. Tunable values ([TUNE]) are defaults; per-platform overrides travel in
//! [`crate::Limits`].

const MS: u64 = 1_000;
const S: u64 = 1_000_000;

// --- neighbor table, NBR (§5.1) ---
/// A link is "good" when `q ≥ Q_GOOD`.
pub const Q_GOOD: u8 = 60;
/// Congested flag hold after a voice drop or a `WouldBlock`.
pub const CONGESTED_HOLD_US: u64 = 2 * S;
/// A neighbor report is fresh when received less than this ago.
pub const NBR_FRESH_US: u64 = 1_500 * MS;
/// NBR on-change rate limit (4 Hz).
pub const NBR_MIN_INTERVAL_US: u64 = 250 * MS;
/// NBR heartbeat.
pub const NBR_HEARTBEAT_US: u64 = S;
/// Link death: no inbound frame for this long.
pub const LINK_DEATH_US: u64 = 4 * S;
/// PING interval while a burst is active (or Lab) / idle.
pub const PING_ACTIVE_US: u64 = 500 * MS;
pub const PING_IDLE_US: u64 = 2 * S;
/// `min_rtt` is the minimum over this window.
pub const MIN_RTT_WINDOW_US: u64 = 30 * S;
/// Loss EWMA weight per second (α = 0.2), in permille.
pub const LOSS_ALPHA_PERMILLE: u32 = 200;

// --- forwarding, TTL, age (§5.2, §5.5) ---
/// Relays drop voice older than this (policy 100-250 ms).
pub const VOICE_MAX_AGE_US: u64 = 150 * MS;
/// Voice TTL at the origin (policy 3-6).
pub const VOICE_TTL: u8 = 4;
/// TTL of the near-talker FLOOR_REJECT (§6.3).
pub const NEAR_TALKER_REJECT_TTL: u8 = 4;
/// Penalty threshold per minute and the resulting ban.
pub const PENALTY_LIMIT_PER_MIN: u32 = 20;
pub const PENALTY_WINDOW_US: u64 = 60 * S;
pub const BAN_US: u64 = 60 * S;
/// Penalties (§5.2).
pub const PENALTY_MALFORMED: u32 = 1;
pub const PENALTY_ORIGIN_SPOOF: u32 = 5;
/// Per-neighbor ingress caps.
pub const INGRESS_RECORDS_PER_S: u32 = 300;
pub const INGRESS_BYTES_PER_S: u32 = 64 * 1024;
/// Control relay jitter range.
pub const CTRL_JITTER_MIN_US: u64 = 5 * MS;
pub const CTRL_JITTER_MAX_US: u64 = 25 * MS;
/// Counter suppression threshold (distinct neighbors).
pub const COUNTER_SUPPRESSION: usize = 3;

// --- dedup (§5.3) ---
/// Window size in sequence numbers.
pub const DEDUP_WINDOW: u16 = 1024;
/// Idle expiry of a window.
pub const DEDUP_EXPIRY_US: u64 = 5 * S;
/// Windows kept per origin.
pub const DEDUP_WINDOWS_PER_ORIGIN: usize = 2;

// --- queues, pacing, airtime (§5.6) ---
/// Voice frames kept per origin per link (drop oldest).
pub const VOICE_FRAMES_PER_ORIGIN: usize = 2;
/// Whole-link voice backlog bound, as drain time at the pace rate.
pub const VOICE_BACKLOG_US: u64 = 60 * MS;
/// P2 share of the pace rate while higher-priority traffic is queued.
pub const P2_SHARE_PERMILLE: u32 = 100;
/// Pacer control period.
pub const PACER_PERIOD_US: u64 = 500 * MS;
/// Queueing-delay thresholds.
pub const QD_HIGH_US: u64 = 40 * MS;
pub const QD_LOW_US: u64 = 15 * MS;
/// Initial pacing rates per kind (L2CAP, GATT, Aware, Tunnel).
pub const R_INIT_L2CAP_BPS: u32 = 60_000;
pub const R_INIT_GATT_BPS: u32 = 40_000;
pub const R_INIT_AWARE_BPS: u32 = 2_000_000;
pub const R_INIT_TUNNEL_BPS: u32 = 64_000;
/// Pacing bounds.
pub const R_MIN_BPS: u32 = 16_000;
pub const R_MAX_BPS: u32 = 4_000_000;
/// Additive increase (BLE, Tunnel) and multiplicative factors (permille).
pub const R_ADD_BPS: u32 = 4_000;
pub const R_DECREASE_PERMILLE: u32 = 850;
pub const R_AWARE_INCREASE_PERMILLE: u32 = 1_100;
/// Node BLE airtime budget [TUNE: T0c] and the idle-link control-only grant.
pub const NODE_BLE_BUDGET_BPS: u32 = 180_000;
pub const R_IDLE_BPS: u32 = 8_000;
/// A BLE link is a voice link if it carried voice within this window.
pub const VOICE_LINK_WINDOW_US: u64 = 2 * S;
/// Cap hysteresis: publish a lower cap after 2 s, a higher one after 5 s.
pub const CAP_DOWN_US: u64 = 2 * S;
pub const CAP_UP_US: u64 = 5 * S;
/// PATH_HINT{BLE12} when `grant < cost(1)` for this long; origins hold BLE12 for 30 s.
pub const PATH_HINT_TRIGGER_US: u64 = 3 * S;
pub const PATH_HINT_HOLD_US: u64 = 30 * S;
/// While the condition persists the relay re-floods PATH_HINT this often, so the
/// origins' 30 s hold never lapses.
pub const PATH_HINT_REFLOOD_US: u64 = 20 * S;

// --- floor (§6.1) ---
pub const S_MAX_BLE: u8 = 2;
pub const S_MAX_AWARE: u8 = 4;
pub const S_MAX_TUNNEL: u8 = 4;
/// Hash smear that makes the order total [TUNE].
pub const T_TIE_US: u64 = 150 * MS;
/// After this an admitted burst is evicted only by higher priority.
pub const T_STICKY_US: u64 = 300 * MS;
/// Slot freed when no voice/TALK of a burst for this long.
pub const FLOOR_IDLE_RELEASE_US: u64 = 600 * MS;
/// Maximum burst length (PTT / VOX).
pub const MAX_BURST_PTT_US: u64 = 60 * S;
pub const MAX_BURST_VOX_US: u64 = 45 * S;
/// TALK KEEP period.
pub const TALK_KEEP_US: u64 = S;
/// Floor table capacity.
pub const FLOOR_TABLE_MAX: usize = 8;
/// Priorities.
pub const PRIO_MEMBER: u8 = 1;
pub const PRIO_LEADER: u8 = 2;
/// Receivers mix at most this many streams.
pub const MIX_STREAMS: usize = 4;

// --- presence, reachability (§5.8) ---
pub const PRESENCE_INTERVAL_US: u64 = 2 * S;
pub const MEMBER_LOST_US: u64 = 6 * S;
pub const ALONE_US: u64 = 10 * S;
pub const PARTITION_STABLE_US: u64 = 6 * S;
pub const EVENT_AGGREGATION_US: u64 = 5 * S;
pub const POSITION_FRESH_US: u64 = 30 * S;
/// Voice reach: KPI within this many BLE link traversals, best effort up to the voice
/// TTL.
pub const KPI_MAX_TRAVERSALS: u8 = 2;

// --- link selection (§5.7, §9.3, §9.5) ---
pub const SELECT_PERIOD_US: u64 = 2 * S;
pub const BLE_LINKS_TARGET: u8 = 2;
pub const BLE_LINKS_MAX: u8 = 3;
pub const AWARE_LINKS_MAX: u8 = 4;
pub const NEIGHBORS_MAX: u8 = 6;
pub const LIMIT_BACKOFF_US: u64 = 10 * 60 * S;
pub const CONNECT_TIMEOUT_US: u64 = 8 * S;
pub const BACKOFF_INITIAL_US: u64 = 5 * S;
pub const BACKOFF_MAX_US: u64 = 60 * S;
pub const SPECULATIVE_INTERVAL_US: u64 = 30 * S;
pub const SPECULATIVE_BACKOFF_US: u64 = 60 * S;
/// HELLO `epoch_lo` 1-4 epochs ahead qualifies for a speculative link.
pub const SPECULATIVE_EPOCHS_AHEAD: core::ops::RangeInclusive<i16> = 1..=4;
