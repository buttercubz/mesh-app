# convLink Voice wire protocol, v1

Human copy of DESIGN §4 (plus the BLE parts of §9.3-§9.4) for people who implement or
review the bytes. The normative references are, in order: the golden vectors in
`test-vectors/wire/` (and `test-vectors/crypto/` for keys, handshakes and signatures), the
`mesh-wire` crate (contract C2), and `docs/DESIGN.md` §4. If this file disagrees with the
vectors, the vectors win and this file is the bug.

Owner: WP-04. Generated artifacts: `cargo xtask vectors` writes `test-vectors/wire/`;
`cargo xtask vectors --check` fails on any byte change.

## 1. General rules

- All multi-byte integers are **little-endian**. Protocol version **1**.
- Every layer is length-delimited and every parser is bounded and total: malformed input
  yields an error, never a panic, never recursion, never decompression.
- Unknown record types are skipped. Unknown mesh packet types are **relayed** but not
  delivered.
- Nothing above the link framing is parsed outside `mesh-wire`. Mesh-level dedup, floor
  and delivery happen only after the SEC frame's AEAD tag verified.

```
transport bytes ─► link framing ─► LinkFrame ─► lf_type ─► Noise HS | SEC (AEAD) | LAB (plain, Lab only) | REJECT
                                                SEC/LAB plaintext ─► records* ─► MESH record ─► MeshPacket
```

## 2. Link framing (§4.2)

| Transport | Unit given to the core | Framing | Max LinkFrame |
|---|---|---|---|
| BLE L2CAP CoC | arbitrary stream chunks | `len u16` + `frame[len]`, `len ∈ [1, 1024]`; an invalid length closes the link (no resync) | 1024 B |
| BLE GATT (fallback) | one ATT value | `hdr u8` + `chunk`; `hdr` b7 START, b6 END, b5-0 frame counter mod 64 (same for all chunks of one frame); chunk ≤ `ATT_MTU − 3 − 1` | 1024 B reassembled |
| Wi-Fi Aware | one datagram | none | 1200 B |
| Tunnel (v2) | stream chunks | as L2CAP | 1024 B |

GATT reassembly: a START discards any partial frame; a chunk whose counter differs from
the partial frame's discards it; the buffer is bounded at 1024 B; END completes the
frame. One frame in flight per direction per link. Chunk payloads per value: 19 B at
MTU 23, 181 B at MTU 185, 513 B at MTU 517.

API: `link::{StreamDeframer, encode_stream, GattChunker, GattReassembler, check_datagram}`.

## 3. LinkFrame (§4.3)

```
offset size field
0      1    lf_type   0x00 HS | 0x01 SEC | 0x02 LAB (Lab links only) | 0x03 REJECT
1      …    body
```

- **HS:** `hs_step u8 (1|2|3)` · `proto_ver u8` · `noise_msg[..]`. Pattern
  `Noise_XXpsk1_25519_ChaChaPoly_SHA256`, prologue `"cvic/v1" ‖ group_id(16) ‖ epoch u32`,
  PSK `psk_e`. Empty-payload sizes: msg1 48 B, msg2 96 B, msg3 64 B. Timeout 5 s.
- **SEC:** `kp_ctr u32` (b31 key phase, b30-0 `ctr`) · `ciphertext` · `tag[16]`.
  IETF ChaCha20-Poly1305 with the raw-split direction key of the current phase,
  nonce `0x00000000 ‖ u64(kp_ctr)` (12 B LE), **AAD `lf_type (0x01) ‖ kp_ctr (4 B LE)`**.
  Overhead 21 B per frame including `lf_type`. `ctr` starts at 0 per direction, link and
  phase. Replay window 1024 bits per direction and phase, updated only after the tag
  verifies.
- **Key update:** after 12 h or `ctr = 2³⁰`: `k_next = HKDF-Expand(PRK = k_cur,
  "cvic/v1/keyupdate", 32)`, flip the phase bit, restart `ctr` at 0. The receiver derives
  `k_next` lazily on the other phase bit, commits only if the tag verifies, keeps the old
  key 10 s.
- **LAB:** `lab_tag u16` · records, where `lab_tag = SHA-256("cvic/lab" ‖ u16 lab_code)[0..2]`
  read little-endian (so the wire bytes are the first two digest bytes). The sender appends
  a `PAD` record with a 16-byte body so the LAB frame has exactly the size of the SEC frame
  of the same records (`records + 21`). Accepted only on links whose HELLO had
  `flags.lab = 1`; never carries group traffic.
- **REJECT:** `reason u8`: 1 version, 2 group/epoch mismatch, 3 at max links, 4 duplicate
  link, 5 member removed/denied, 6 lab code mismatch, 7 handshake failure, 8 node_id
  collision. The sender then closes.

## 4. Records (§4.4)

`rtype u8` · `len` LEB128 (1-2 B, shortest form, ≤ 16383) · `body[len]`. At most 32
records per frame; the sender coalesces whatever is queued and never waits to coalesce.

| rtype | Name | Body | Size |
|---|---|---|---|
| 0x01 | MESH | MeshPacket (§5) | 9 + payload |
| 0x02 | PING | `id u16, t_tx u32` (µs, truncated) | 6 |
| 0x03 | PONG | `id u16, t_tx u32` (echo), `hold_us u16` | 8 |
| 0x04 | NBR | `ver u8=1, nseq u8, count u8 (≤ 8), count × (node_id u16, q u8, flags u8, vcap u8)`; flags b0 good, b1 congested, b3-2 kind (0 BLE, 1 Aware, 2 Tunnel) | 3 + 5n |
| 0x05 | LSTAT | `rx_frames u16, rx_bytes u32, gaps u16, pace_kbps u16, queue_ms u8` | 11 |
| 0x06 | CAPS | `proto u8, platform u8 (0 ios, 1 android, 2 websim), app_ver u16, codec u16 (b0 opus 1.6.1, b1 DRED, b2 OSCE, b3 reserved), transports u8 (b0 l2cap, b1 gatt, b2 aware-ios, b3 aware-android, b4 tunnel), max_links u8, role u8, epoch u32, roster_version u32, ctl_version u32` (zeros in the summary on Lab links) | 21 |
| 0x07 | LPROBE | `probe_id u32, t_tx u64, flags u8, pad[..]` (zeros) | 13 + pad |
| 0x08 | LECHO | `probe_id u32, t_tx u64, hold_us u32, rx_count u32` | 20 |
| 0x09 | UPGRADE | `op u8 (1 offer … 5 retire), transport u8 (1 aware-ios, 2 aware-android, 3 tunnel), nonce u32, params[..]` | 6+ |
| 0x0A | CLOSE | `reason u8` (REJECT reasons plus 9 redundant, 10 leaving) | 1 |
| 0x0B | LAB_CTL | `op u8 (1 start, 2 stop, 3 mark, 4 pattern, 5 announce), run_id u32, bench u8, params[..]`; announce params = `lab_node_id u16, nonce u64` | 6+ |
| 0x0C | IDPROOF | `ed_pk[32], x_pk[32], sig[64]`; sig = Ed25519 over `"cvic/v1/idproof" ‖ role u8 (0 initiator, 1 responder) ‖ handshake_hash(32)` | 128 |
| 0x0D | LAB_GPS | `t_ms u32, lat_e7 i32, lon_e7 i32, speed_cms u16, acc_dm u16, heading_cdeg u16` | 18 |
| 0x0E | PAD | zero bytes, ignored | n |
| 0x0F | SYNC | `kind u8 (1 ROSTER, 2 REKEY, 3 GROUP_CTL, 4 JOIN_REQ), msg_id u16, seg u8 (idx<<4 \| count−1), chunk[≤ 900]` | 4 + chunk |
| 0x10-0xFF | reserved, skipped | | |

Fixed-size bodies must have exactly their size. SYNC chunks of one logical body are
reassembled per link and `msg_id` (≤ 2 concurrent per link, 5 s timeout) and then handled
exactly like the flooded message.

API: `record::{Record, RecordIter, RecordWriter, encode_records}`.

## 5. MeshPacket header (§4.5)

```
offset size field
0      1    vt        version (b7-5, = 1) | ptype (b4-0)
1      1    ttl_hops  ttl_remaining (b7-4) | hops (b3-0, saturating 15)
2      2    origin    node_id u16
4      1    sess      random non-zero u8 per engine start
5      2    seq       per-(origin, sess) u16, serial arithmetic, random start
7      1    age       2 ms units since creation, saturating 255 (= 510 ms)
8      1    flags     b7-6 class (0 ctrl, 1 voice, 2 bulk, 3 lab) | b5 LEADER | b4 SEG | b3-0 reserved (0)
9      …    payload
```

Relay rewrite (inside the link encryption): `ttl −= 1`, `hops += 1` (saturating),
`age += local residence + incoming one-way estimate` (rounded to 2 ms units). TTL 0 is
not forwarded. Dedup key `(origin, sess, seq)`.

API: `mesh::{MeshHeader, MeshPacket, PacketType, TrafficClass}`.

## 6. Mesh packet types (§4.6)

| ptype | Name | Payload | Bytes | TTL | Class |
|---|---|---|---|---|---|
| 0x01 | VOICE | `burst u8 · ts u16 · cfg u8 · opus[1..=400]`; `ts` = 10 ms ticks since the burst's first VOICE frame (frame n of a 40 ms profile has `ts = 4n`); `cfg` b7-6 dur (0=10/1=20/2=40/3=60 ms), b5-4 bw (0 NB, 1 MB, 2 WB, 3 reserved), b3 LBRR, b2 DRED, b1 FIRST, b0 PREROLL | 4 + opus | 4 (policy 3-6) | voice |
| 0x02 | TALK | `burst u8, state u8 (1 START, 2 STOP, 3 KEEP every 1 s), prio u8 (1 member, 2 leader), mode u8 (1 vox, 2 ptt), profile u8 (1 BLE16, 2 BLE12, 3 WIFI20, 4 WIFI24FEC, 5 BLE-DRED), elapsed_10ms u16` (0 on START) | 7 | 6 | ctrl |
| 0x03 | FLOOR_REJECT | `target u16, target_sess u8, burst u8, reason u8 (1 cap, 2 preempted, 3 congestion, 4 outranked), at u16` | 7 | 6 | ctrl |
| 0x04 | PRESENCE | §6.1 | 14 | 15 | ctrl |
| 0x05 | ROSTER | segmented (§7) | ≤ 16 seg | 15 | ctrl |
| 0x06 | JOIN_REQ | segmented | 1 seg | 15 | ctrl |
| 0x07 | REKEY | segmented | ≤ 16 seg | 15 | ctrl |
| 0x08 | LEAVE | `reason u8 (1 user, 2 removed, 3 app-exit), sig[64]` over `"cvic/v1/leave" ‖ group_id ‖ origin u16 ‖ sess u8 ‖ reason u8` | 65 | 15 | ctrl |
| 0x09 | RIDE_CTL | `op u8 (1 start, 2 end), ride_id u32, sig[64]` over `"cvic/v1/ridectl" ‖ group_id ‖ epoch u32 ‖ ride_id u32 ‖ op u8` | 69 | 15 | ctrl |
| 0x0A | MESH_PROBE | `probe_id u32, t_origin u64, n u8 (≤ 8), trace[n] u16` | 13 + 2n | 8 | lab |
| 0x0B | PATH_HINT | `profile u8 (1 BLE16, 2 BLE12), at u16, target u16 (0 = all)` | 5 | 3 | ctrl |
| 0x0C | GROUP_CTL | `op u8 (1 close, 2 transfer-leader), ctl_version u32, epoch u32, [new_leader_ed_pk[32], new_leader_node u16 if op 2], sig[64]` over `"cvic/v1/groupctl" ‖ group_id ‖ preceding bytes` | 73 / 107 | 15 | ctrl |
| 0x10 | POSITION | `lat_e7 i32, lon_e7 i32, speed_dms u16, heading_2deg u8, acc_m u8, fix_age_100ms u8, rsv u8` | 14 | 15 | bulk |
| 0x11 | ALERT | reserved | — | — | ctrl |
| 0x1F | EXT | `ext_type u8` + body | — | — | — |

Signatures are made and checked by mesh-crypto; mesh-wire builds the exact signed
messages (`Leave::signed_message`, `RideCtl::signed_message`, `GroupCtl::signed_message`).
API: `mesh::{MeshPayload, encode_packet, Voice, Talk, FloorReject, Presence, Leave, RideCtl,
MeshProbe, PathHint, GroupCtl, Position, Ext}`.

### 6.1 PRESENCE (every 2 s, 14 B)

```
roster_ver16 u16 · epoch16 u16
state  u8  b7-6 ride (0 lobby, 1 riding, 2 ending) | b5-4 talk (0 idle, 1 talking, 2 blocked, 3 muted)
           | b3 leader | b2 on_call | b1 capture_ok | b0 live
audio  u8  b7-6 ladder (L0-L3) | b5-3 route (0 none, 1 builtin, 2 hfp-nb, 3 hfp-wb, 4 le-audio, 5 car-hfp, 6 wired)
           | b2 tx_muted_no_headset | b1 render_ok | b0 sharing_distance
battery u8 (255 unknown) · links u8 (b7-4 BLE, b3-0 Aware)
caps   u8  b7-6 platform | b5 dred_ok | b4 radio_off_recently (BT off within 60 s) | b3-0 reserved
interval u8 (100 ms units) · heard u32 (bitmap by roster index)
```

**Truncated version fields.** `roster_version` and `epoch` are u32 wherever a message is
authoritative (ROSTER, REKEY, GROUP_CTL, CAPS, invite). PRESENCE `roster_ver16`/`epoch16`
and HELLO `epoch_lo` carry `value mod 2¹⁶` and are compared with RFC 1982 serial
arithmetic (`serial::{truncate16, serial16_cmp, peer_behind, ahead_by}`).

**Names** are UTF-8 bytes; `name_len` counts bytes; encoders truncate at the last
extended grapheme-cluster boundary that fits: display name ≤ 24 B, group name ≤ 32 B
(`name::truncate_graphemes`).

### 6.2 Segmentation (SEG flag)

The payload starts with `msg_id u16` · `seg u8 = idx<<4 | (count−1)` (≤ 16 segments,
≤ 900 B of data each). Each segment is its own MeshPacket with its own `seq` and is
deduplicated individually; relays forward segments without reassembly. Receivers
reassemble by `(origin, sess, msg_id)` with ≤ 4 concurrent assemblies per origin, a 5 s
timeout and ≤ 16 × 900 B (`mesh::{segment_packets, SegmentAssembler}`).

## 7. Control message bodies (§4.7, logical form before segmentation)

- **ROSTER:** `ver u8=1 · group_id[16] · epoch u32 · roster_version u32 · invite_sig[64] ·
  n u8 · n × (node_id u16 · id_hash[16] · ed_pk[32] · role u8 (0 member, 1 leader, 0xFF
  denied) · name_len u8 · name[≤ 24 B]) · sig[64]`; leader signature over
  `"cvic/v1/roster" ‖ preceding bytes`. Roster index = position among non-denied entries
  (0-31, at most 32 members).
- **JOIN_REQ:** `ver u8=1 · ed_pk[32] · x_pk[32] · platform u8 · name_len u8 · name[≤ 24 B] ·
  sig[64]`, signature over `"cvic/v1/join" ‖ group_id ‖ preceding bytes`.
- **REKEY:** `new_epoch u32 · roster_version u32 · leader_eph_pk[32] · n u8 · n × (node_id
  u16 · sealed[48]) · sig[64]`; leader signature over `"cvic/v1/rekey" ‖ group_id ‖
  preceding bytes` (the context string is fixed by mesh-wire; §4.7 leaves it implicit).
- Acceptance and monotonicity (ROSTER strictly newer, REKEY newer epoch with an entry for
  me, GROUP_CTL strictly newer `ctl_version` and `epoch ≥` current) are mesh-crypto rules.

API: `mesh::{Roster, RosterEntry, RosterRole, JoinReq, Rekey, RekeyEntry, ControlBody}`.

## 8. Sizes and cost model (§4.8)

| Item | Bytes / rate |
|---|---|
| BLE VOICE 16 kbps, 40 ms (Opus 80 B) | MeshPacket 93 → record 95 → SEC 116 → L2CAP 118 B ⇒ 23.6 kbps |
| BLE-DRED 20 kbps, 40 ms (Opus 100 B) | 113 → 115 → 136 → 138 B ⇒ 27.6 kbps |
| BLE12 12 kbps, 40 ms (Opus 60 B) | 73 → 75 → 96 → 98 B ⇒ 19.6 kbps |
| Wi-Fi 20 kbps, 20 ms (Opus 50 B) | 63 → 65 → 86 B datagram ⇒ 34.4 kbps |
| Two BLE16 streams | 47.2 kbps uncoalesced (design value), 42.6 kbps coalesced |
| PRESENCE / POSITION record | 25 B each |
| `C_CTRL` | 3 kbps per link |

`cost(k, profile) = k × r_stream(profile) + C_CTRL` (uncoalesced rates); `cap_pacing` =
the largest `k` with `cost(k) ≤ 0.9 × grant`. BLE16 at S = 2: 50.2 kbps ≤ 54 kbps.
API: `budget::{voice_sizes, r_stream_bps, cost_bps, cap_pacing}`.

## 9. BLE identifiers (§9.3-§9.4)

| Item | UUID |
|---|---|
| Service `MESH_SVC` | `31B8CD85-9D3A-4622-A465-3CDCA2C32DC5` |
| `HELLO` (read, unencrypted, 16 B) | `86841842-A7C3-4844-84D2-5B281A3F616C` |
| `RX` (write without response) | `14E91943-7673-440A-979A-F85B4DBD4DBF` |
| `TX` (notify) | `A76E3168-B4F6-4BE1-8252-35AB8AF17302` |

**HELLO value (16 B):** `magic u8 = 0xC1 · proto_min u8 · proto_max u8 · flags u8 (b0
android, b1 lab, b2 accepting, b3 aware-capable, b4 leader) · psm u16 (0 ⇒ GATT only) ·
group_tag[4] (Lab: lab_tag u16 LE ‖ 0x0000) · node_token[4] · epoch_lo u16`. The initiator
picks the highest common version (`Hello::common_version`); a peer whose `epoch_lo` is 1-4
ahead is a speculative-link candidate (`Hello::speculative_candidate`).

**Advertising:**

- Android primary ADV (21 B): Flags `02 01 06` + Complete List of 128-bit UUIDs
  `11 07 <MESH_SVC LE>`.
- Android scan response (27 B): Service Data 128-bit `1A 21 <MESH_SVC LE>` + `flags u8
  (b0 android, b1 lab) · group_tag[4] · node_token[4]`.
- iPhone foreground local name: 6 characters, RFC 4648 base32 (`A-Z2-7`, MSB first) of the
  30-bit value `group_tag[0] ‖ group_tag[1] ‖ top 14 bits of node_token`; AD `07 09 <name>`.
- Initiator rules (§9.2): iPhone↔iPhone foreground, the lower 14-bit token prefix initiates
  (equal prefixes: either side); Android↔Android, the lower `node_token`; duplicates after
  IDPROOF keep the link whose initiator has the lower `node_id`.

Timings and limits are exported by `consts::ble_constants()` (connect 8 s, handshake 5 s,
backoff 5 → 60 s, link death 4 s, PING 500 ms / 2 s, adv 250 ms / 1 s, scan restart 9 min,
5 starts per 30 s, token rotation 600 s, links 2 / 3, neighbors 6, MTU 517).

## 10. Golden vectors

`test-vectors/wire/*.json`, one file per group:

| File | Content |
|---|---|
| `link.json` | L2CAP frames, GATT chunking at MTU 23/185/517 (1-1024 B frames, counter wrap), datagram |
| `frame.json` | HS msg1-3, SEC in both phases with AAD and nonce, `lab_tag`, padded LAB, every REJECT, Noise prologue |
| `records.json` | every record 0x01-0x0F, LEB128 forms, unknown record, coalesced plaintext, IDPROOF signed message |
| `mesh.json` | header, relay rewrite, every ptype as a full packet, signed messages of LEAVE/RIDE_CTL/GROUP_CTL |
| `segmentation.json` | ROSTER (32 entries, 3 segments, seq wrap), JOIN_REQ, REKEY, SYNC chunks |
| `control.json` | ROSTER/JOIN_REQ/REKEY bodies and their signed messages |
| `hello.json` | HELLO values (group and Lab) |
| `adv.json` | UUID bytes, Android ADV and scan response, iPhone local names |
| `names.json` | grapheme-boundary truncation |
| `sizes.json` | the §4.8 byte chain and rates |

File format: `{"format": 1, "protocol": 1, "description": "...", "vectors": [{"name",
"inputs", "expected"}]}`; byte strings are lowercase hex; `expected` is one hex string, or
an array of hex strings when one input yields several units (GATT values, segments, SYNC
records). Byte strings named in `inputs` come from a fixed filler pattern; signatures and
sealed boxes are filler of the right length (real keys, handshakes, sealed frames and
signatures are in `test-vectors/crypto/`, generated by mesh-crypto through the same
`cargo xtask vectors` command). Swift and Kotlin tests replay `hello.json` and `adv.json`.

## 11. Fuzzing

`fuzz/` (cargo-fuzz, nightly only): `stream_deframer`, `gatt_reassembly`, `link_frame`,
`records`, `mesh_header`, `mesh_ptypes`, `segmentation`, `control_bodies`, `hello_adv`.
Each target asserts round-trip properties, not only absence of panics. The invite URL and
Noise handshake-input targets of DESIGN §13.1 are added once mesh-crypto (WP-07) exposes
those APIs. Run: `cd fuzz && cargo +nightly fuzz build`, then
`cargo +nightly fuzz run <target> -- -max_total_time=300`.
