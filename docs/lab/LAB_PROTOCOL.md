# Lab protocol — field benches / Protocolo de laboratorio — bancos de campo

Owner: WP-05 (DESIGN §12). English first, then Spanish ([ir a la versión en español](#versión-en-español)).
The Field test screen (Settings → Field test) shows this text per bench.

---

## English

### Safety first

- **Riders never operate the phone while moving.** The Start, Stop and Mark buttons are
  for **passengers or at stops**. If nobody can press Mark safely, skip the mark: the
  1 Hz samples still record speed, distance and link state.
- Mount the phone before the run starts. Do not reach for a pocket or tank bag while riding.
- Benches on public roads follow traffic law; nobody rides differently "for the test".

### What every run needs

1. Every phone opens **Settings → Field test** and enters the same **4-digit Lab code**.
   No group is needed. Phones only talk to phones with the same code (the code becomes
   `lab_tag`; a different code is refused with REJECT 6).
2. One phone is the **controller**: it picks the bench and its variant and taps Start.
   It sends `LAB_CTL{start}` with the run parameters; the other phones join as
   **followers** and record the same `runId` (`r-YYYYMMDD-HHMM-xxxx`).
3. Check the config sheet before Start:
   - **Audio path** (mandatory): `off` (radio only), `renderOnly` (session + output:
     keeps the audio anchor, plays bench tones, no mic) or `full` (capture + render,
     HFP when a headset is connected). The bench default is pre-selected (table below).
   - **Keep-alive anchor** (mandatory): `audio` (default), `audioLocation` (needs the
     Location permission, asked in the foreground now) or `none` — labelled
     "control: the phone may suspend". `none` is never a default.
   - **GPS consent** for this run (needed for distance in T1/T6 and the ride recorder).
4. Tap **Start** while the screen is on. With audio ≠ off, Start is the gesture that
   opens the audio session (same rules as Go live). Keep the screen awake until the run
   has started, then lock the phone if the variant says so.
5. During the run: press **Mark** (passengers / stops only) to label moments such as
   "pocket", "helmet off", "tunnel". Write down anything unusual on paper.
6. The run stops by itself after its duration, or with Stop (controller) — the
   followers stop with it.
7. **Export:** results list → share sheet → AirDrop to the Mac. Each phone writes one
   `labrun/1` JSONL file per run (`Documents/lab` on iOS, visible in Files) plus a
   one-row `…-summary.csv`.
8. **Analyse on the Mac:** put every phone's files of a session in one folder, then
   `node tools/lab-analyze/cli.mjs merge <folder>` (CSV in `<folder>/analysis`) and
   `node tools/lab-analyze/cli.mjs verdicts <folder>`. `validate <folder>` checks the
   files against the schema first.

### Honesty rules

- Automated results and **pending physical** items are reported separately. Nothing
  physical is ever faked; a run that did not happen stays "pending".
- A run whose anchor dropped is **excluded**, not counted as radio loss (T1); the
  analysis reports how many windows were excluded and why.
- Keep failed and aborted runs: an interrupted file (no summary line) is still analysed
  from its 1 Hz samples and is marked as such.

### Benches (defaults from DESIGN §12.2)

| Bench | Phase | Phones | Audio / anchor (default) | Variants (each one run) | Go / no-go |
|---|---|---|---|---|---|
| **T0** BLE throughput / latency / loss | 0 | 2 at 1-5 m, then 20 m | `off` (headset off) or `full` (headset on) / `audio` | transport {l2cap, gattNotify, gattWrite} × role {central, peripheral} × headset {off, hfp} × pattern {saturate 20 s, voice1-voice4 60 s, ping 64 B @ 10 Hz, ping under load} × distance {near, far} | ≥ 2 voice-like streams at < 5 % loss and p95 one-way < 60 ms **with the headset active** ⇒ S_MAX_BLE = 2; ≥ 150 kbps ⇒ BLE-fast |
| **T0b** relay chain | 0 | 3 (A–B–C, A and C out of range or blacklisted) | `off`, `full` on B with headset / `audio` | headset on B {off, hfp} × {voice1, voice2} | 2 streams relayed at < 5 % loss, added p95 per hop ≤ 40 ms |
| **T0c** multi-link hub | 0 | 4-5 (3 → hub with 2 links) | `full` / `audio` | links on H {2, 3, 4} × pacer {free, capped} | target 3 / max 4 only if H sustains 3 links × cost(2) at < 5 % loss, HFP glitch-free |
| **T1** range between vehicles | 0 | 2-3 vehicles | `renderOnly` / `audio` | mount {mounted, pocket, tankBag} × anchor {audio, audioLocation}; 30-60 min | ≥ 95 % of valid 1 s windows < 5 % loss up to the typical convoy gap; anchor-dropped windows excluded |
| **T2** background survival | 0 | 1 (+1 peer) | `full` / `audio` | anchor {audio, audioLocation, none} × session {nonMixable, mixable}; 4 h; long run past the 8 h Live Activity limit | 0 silent mutes; automatic recovery or one guided tap |
| **T3** rediscovery | 0 (Android variant 3) | 2 (3 for relay rejoin) | `renderOnly` / `audio` (`none` control) | state {bothFg, oneBg, bothBgScreenOff, bothBgLiveActivity, pendingConnect, behindEpoch} × break {blacklist30s, walkApart}; 5 trials each; **[DEV] Phase 3:** Android scanner finds a backgrounded iPhone via the overflow bitmask | rejoin ≤ 10 s in the variants the product uses |
| **T5a** headset loopback latency | 0 | 1 + helmet | `full` / `audio` | per headset model × HFP {wideband, narrowband}; VPIO `BypassVoiceProcessing = 1`; 50 click trains | per-leg estimate ≤ 25 ms |
| **T5b** headset behaviour | 1 | 1-2 + each helmet | `full` / `audio` | check {echoReturn, ducking, buttons, helmetIntercom} | per-model note in the Phase 1 report |
| **T6** battery and thermal | 0 (≥ 1 h per variant) | 1-2 on a real ride | `full` / per variant | screen {off, offNav, onMount} × charging {no, yes} × anchor {audio, audioLocation} | records %/h; proposed target < 5 %/h screen off |
| **T8** UI / PTT input latency | 1 | 1, Demo world (15 peers) | `full` / `audio` | 100 PTT presses | touch → `mesh_rt_set_flags` p95 < 50 ms, no frame > 50 ms |
| **VOX-cal** | 1 | 1 rider | `full` / `audio` | sensitivity {low, medium, high} × wind {calm, windy} | ≤ 1 false trigger/min at 80 km/h, onset ≤ 150 ms at Medium |
| **T4** Wi-Fi Aware | 4 | 2-3 paired iPhones | `off` / `audio`; battery variants `full` | mode {realtime, bulk} × topology {singleRole, bothRoles}; battery: realtime vs BLE-only (1 h each) | worth it if ≥ 4 streams at < 2 % loss; battery cost recorded |

### Per-bench procedure notes

- **T0:** start at 1-5 m in the open, phones on a table, screens on. Repeat the matrix
  at 20 m. For headset-on variants, connect the helmet first and confirm the route
  says HFP before Start. Record the helmet model in the variant label.
- **T0b:** force A and C apart (distance or `LAB_CTL` blacklist) so the only path is via
  B. The controller (A) floods MESH_PROBE at 5 Hz; B and C record the trace.
- **T0c:** the hub H runs the HFP headset. Add links one at a time; note any audible
  glitch with a Mark (passenger) — `mesh_rt_stats` counts underruns automatically.
- **T1:** both vehicles run the same variant. Mark mount changes and stops. A run is
  useful only with GPS consent on both phones (distance comes from the peers'
  LAB_GPS at 1 Hz).
- **T2:** lock the phone and follow the script on paper: helmet off/on, incoming call,
  Siri, music app, navigation prompt, Low Power Mode, Bluetooth off/on, TTS prompt while
  locked. Every capture stop must produce a CaptureLost notification.
- **T3:** break the link with the blacklist for 30 s or by walking apart; 5 trials per
  variant. The analysis reads `linkUp.reconnectS`.
- **T5a:** quiet room, helmet speaker against the helmet mic. The core plays a 256 ms MLS
  click train every 2 s and finds it in the capture; 50 repetitions.
- **T6:** start with the battery between 90 % and 30 %, not charging unless the variant
  says so. ≥ 1 h per variant in Phase 0.

### Technical notes (for developers)

- Traffic is generated by `mesh-lab` inside the core (same code on iOS, Android and the
  simulator). Voice-like probes are padded so one LAB frame has exactly the on-air size
  of a BLE16 VOICE frame (118 B on L2CAP).
- **LPROBE flags** (Lab only): bits 1-0 class (0 data, 1 ping = echo requested, 2 reply),
  bits 4-2 voice-like stream index. A ping is answered in one frame by a LECHO
  (`t_tx` echo, `hold_us`, `rx_count`) and a reply LPROBE with the same `probe_id` and the
  responder's transmit time — the four NTP timestamps for the clock-offset estimator
  (minimum-RTT filter over the last 256 exchanges).
- **LAB_CTL `start` params:** `ver u8=1 · audio u8 · anchor u8 · flags u8 (b0 gps, b1
  headset, b2 mixable) · duration_s u32 · pattern u8 · streams u8 · payload_b u16 ·
  interval_ms u16 · ping_hz u8 · transport u8 · role u8 · vlen u8 · variant[vlen]`;
  the `bench` byte is T0=0, T0b=1, T0c=2, T1=3, T2=4, T3=5, T4=6, T5a=7, T5b=8, T6=9,
  T8=10, voxCal=11, ride=12.
- File format: `labrun/1`, schema `crates/mesh-lab/schema/labrun-1.schema.json`. The run
  line always carries `audio` and `anchor`; every 1 Hz sample carries `anchor` (anchor
  state) and `bt` (Bluetooth radio state).
- Thresholds live in `tools/lab-analyze/thresholds.json`. `T1.convoyGapM` (50 m) is a
  placeholder until the typical convoy gap is decided (DESIGN Appendix E #1).

### Ride diagnostics (`ridelog/1`)

The always-on flight recorder of every phone (not a bench). Settings → Diagnostics turns
it on or off (on by default while the app version is below 1.0), exports the last ride or
everything as `convlink-diag-<rideKey>-n<node>-<date>.zip` through the share sheet, and
lets the rider mark a problem with a fixed chip (crackle, can't hear, not heard, delay,
someone dropped, other). The zip holds `device.json` and `diag/<sessionId>/seg-NNNN.jsonl`.

- **Schema:** `crates/mesh-lab/schema/ridelog-1.schema.json` (Rust writer and validator
  test in `crates/mesh-lab/src/ridelog/` and `tests/ridelog_schema.rs`). Lines: `hdr`
  (first line of every segment: device, session id `d-YYYYMMDD-HHMM-xxxx`, `rideKey`,
  node, clock anchor), `s` (1 Hz; counters are **per-second deltas**, gauges as is), `e`
  (events, `src` core/rt/plat/ui), `mark`, `end` (segment closed: rotate, session end,
  app terminate), `f` (verbose per-frame, reserved). `ts` = seconds since the session
  start on that phone.
- **rideKey:** first 4 bytes (hex) of `SHA-256("ridelog" ‖ group_id ‖ u32le UTC
  YYYYMMDD)`; `lab-<lab tag>` in Lab mode, `demo` for the Demo ride. All phones of one
  ride on one UTC day share it; it reveals nothing about the group.
- **Privacy:** no keys, invites, names, MAC addresses, audio or exact GPS; strings longer
  than 64 characters or key-like (24+ hex / 32+ base64 characters) are replaced by
  `<redacted>` in the runtime. Files stay in app-private storage (iOS Application
  Support, excluded from backup; Android `filesDir`) until exported. Budget 64 MiB,
  oldest sessions first, never the current one; 14 days retention; segments rotate at
  1 MiB or 10 min.
- **Merge and report:** `node tools/lab-analyze/cli.mjs ride <zip|dir|jsonl...>
  [--ride <rideKey>] [--from HH:MM --to HH:MM] [--all] [--json] [--csv <dir>]
  [--as-labrun <dir>]`. It groups the phones by `rideKey`, aligns their clocks (wall
  clock, refined by `clkOffset` events where `offsetUs` = peer engine clock − own engine
  clock), and prints the merged timeline (UTC), the per-link traffic per minute with both
  ends side by side and the **idle overhead** (non-voice kbit/s per phone), the audio
  glitch summary (underruns, callback-size changes, late/PLC/FEC per 1000 frames, mic
  state transitions and the "permission granted but mic paused" check), partitions and
  who hears whom, and each mark with ±10 s of context. Any line that fails the schema
  makes it exit 1. `--as-labrun` writes `labrun/1` (bench `ride`) files for the Lab
  analysis.

---

## Versión en español

### Primero, la seguridad

- **Quien conduce nunca manipula el teléfono en marcha.** Los botones Iniciar, Detener
  y Marca son para **acompañantes o para las paradas**. Si nadie puede marcar con
  seguridad, no se marca: las muestras de 1 Hz ya registran velocidad, distancia y
  estado del enlace.
- Fija el teléfono antes de empezar. No busques el teléfono en el bolsillo o en la
  bolsa del tanque mientras conduces.
- En vía pública se respetan las normas de tránsito; nadie conduce distinto "por la prueba".

### Lo que necesita cada corrida

1. Cada teléfono abre **Ajustes → Prueba de campo** e introduce el mismo **código de
   laboratorio de 4 dígitos**. No hace falta grupo. Solo se conectan teléfonos con el
   mismo código (el código se convierte en `lab_tag`; uno distinto se rechaza con REJECT 6).
2. Un teléfono es el **controlador**: elige el banco y su variante y pulsa Iniciar.
   Envía `LAB_CTL{start}` con los parámetros; los demás se unen como **seguidores** y
   graban el mismo `runId` (`r-AAAAMMDD-HHMM-xxxx`).
3. Revisa la hoja de configuración antes de Iniciar:
   - **Ruta de audio** (obligatoria): `off` (solo radio), `renderOnly` (sesión + salida:
     mantiene el ancla de audio, reproduce tonos, sin micrófono) o `full` (captura +
     reproducción, HFP si hay casco conectado). Viene preseleccionada la del banco.
   - **Ancla de permanencia** (obligatoria): `audio` (por defecto), `audioLocation`
     (requiere el permiso de Ubicación, que se pide ahora en primer plano) o `none` —
     rotulada "control: el teléfono puede suspenderse". `none` nunca es la opción por defecto.
   - **Consentimiento de GPS** para esta corrida (necesario para la distancia en T1/T6 y
     en el grabador de rodadas).
4. Pulsa **Iniciar** con la pantalla encendida. Con audio distinto de off, Iniciar es el
   gesto que abre la sesión de audio (mismas reglas que "Salir en vivo"). Mantén la
   pantalla encendida hasta que la corrida empiece y luego bloquea el teléfono si la
   variante lo pide.
5. Durante la corrida: pulsa **Marca** (solo acompañantes o paradas) para etiquetar
   momentos como "bolsillo", "casco fuera", "túnel". Anota en papel lo que sea raro.
6. La corrida termina sola al cumplir su duración, o con Detener (controlador); los
   seguidores se detienen con él.
7. **Exportar:** lista de resultados → compartir → AirDrop al Mac. Cada teléfono escribe
   un archivo JSONL `labrun/1` por corrida (`Documents/lab` en iOS, visible en Archivos)
   y un `…-summary.csv` de una fila.
8. **Analizar en el Mac:** pon los archivos de todos los teléfonos de una sesión en una
   carpeta y ejecuta `node tools/lab-analyze/cli.mjs merge <carpeta>` (CSV en
   `<carpeta>/analysis`) y `node tools/lab-analyze/cli.mjs verdicts <carpeta>`.
   `validate <carpeta>` comprueba antes los archivos contra el esquema.

### Reglas de honestidad

- Los resultados automáticos y lo **pendiente físico** se informan por separado. Nada
  físico se inventa; una corrida que no ocurrió queda "pendiente".
- Una corrida cuyo ancla se cayó se **excluye**, no se cuenta como pérdida de radio
  (T1); el análisis dice cuántas ventanas se excluyeron y por qué.
- Conserva las corridas fallidas o abortadas: un archivo interrumpido (sin línea de
  resumen) se analiza igual a partir de sus muestras de 1 Hz y se marca como tal.

### Bancos (valores por defecto de DESIGN §12.2)

| Banco | Fase | Teléfonos | Audio / ancla (por defecto) | Variantes (cada una, una corrida) | Va / no va |
|---|---|---|---|---|---|
| **T0** rendimiento / latencia / pérdida BLE | 0 | 2 a 1-5 m, luego 20 m | `off` (sin casco) o `full` (con casco) / `audio` | transporte {l2cap, gattNotify, gattWrite} × rol {central, periférico} × casco {off, hfp} × patrón {saturate 20 s, voice1-voice4 60 s, ping 64 B a 10 Hz, ping con carga} × distancia {cerca, lejos} | ≥ 2 flujos tipo voz con < 5 % de pérdida y p95 de un sentido < 60 ms **con el casco activo** ⇒ S_MAX_BLE = 2; ≥ 150 kbps ⇒ BLE rápido |
| **T0b** cadena de relevo | 0 | 3 (A–B–C, A y C fuera de alcance o en lista negra) | `off`, `full` en B con casco / `audio` | casco en B {off, hfp} × {voice1, voice2} | 2 flujos relevados con < 5 % de pérdida, p95 añadido por salto ≤ 40 ms |
| **T0c** concentrador multienlace | 0 | 4-5 (con 3, el concentrador lleva 2 enlaces) | `full` / `audio` | enlaces en H {2, 3, 4} × marcapasos {libre, limitado} | objetivo 3 / máx. 4 solo si H sostiene 3 enlaces × cost(2) con < 5 % de pérdida y HFP sin cortes |
| **T1** alcance entre vehículos | 0 | 2-3 vehículos | `renderOnly` / `audio` | montaje {soporte, bolsillo, bolsa de tanque} × ancla {audio, audioLocation}; 30-60 min | ≥ 95 % de ventanas válidas de 1 s con < 5 % de pérdida hasta la separación típica del convoy; ventanas con el ancla caída se excluyen |
| **T2** supervivencia en segundo plano | 0 | 1 (+1 par) | `full` / `audio` | ancla {audio, audioLocation, none} × sesión {no mezclable, mezclable}; 4 h; corrida larga más allá del límite de 8 h de la Actividad en vivo | 0 silencios mudos; recuperación automática o un solo toque guiado |
| **T3** redescubrimiento | 0 (variante Android: 3) | 2 (3 para reingreso por relevo) | `renderOnly` / `audio` (`none` control) | estado {ambosPrimerPlano, unoFondo, ambosFondoPantallaApagada, ambosFondoActividad, conexiónPendiente, épocaAtrasada} × corte {listaNegra30s, alejarse}; 5 intentos cada una; **[DEV] Fase 3:** el escáner Android encuentra un iPhone en segundo plano por el bitmask de desbordamiento | reingreso ≤ 10 s en las variantes que usa el producto |
| **T5a** latencia de lazo del casco | 0 | 1 + casco | `full` / `audio` | por modelo de casco × HFP {banda ancha, banda estrecha}; VPIO `BypassVoiceProcessing = 1`; 50 trenes de clics | estimación por tramo ≤ 25 ms |
| **T5b** comportamiento del casco | 1 | 1-2 + cada casco | `full` / `audio` | prueba {retorno de eco, atenuación, botones, intercomunicador del casco} | nota por modelo en el informe de Fase 1 |
| **T6** batería y temperatura | 0 (≥ 1 h por variante) | 1-2 en una rodada real | `full` / según variante | pantalla {apagada, apagada + navegación, encendida en soporte} × cargando {no, sí} × ancla {audio, audioLocation} | registra %/h; objetivo propuesto < 5 %/h con pantalla apagada |
| **T8** latencia de UI / PTT | 1 | 1, mundo Demo (15 pares) | `full` / `audio` | 100 pulsaciones de PTT | toque → `mesh_rt_set_flags` p95 < 50 ms, ningún cuadro > 50 ms |
| **Calibración VOX** | 1 | 1 motorista | `full` / `audio` | sensibilidad {baja, media, alta} × viento {calma, ventoso} | ≤ 1 disparo falso/min a 80 km/h, inicio ≤ 150 ms en Media |
| **T4** Wi-Fi Aware | 4 | 2-3 iPhones emparejados | `off` / `audio`; variantes de batería `full` | modo {tiempo real, masivo} × topología {un rol, ambos roles}; batería: tiempo real vs solo BLE (1 h cada una) | vale la pena si ≥ 4 flujos con < 2 % de pérdida; se registra el costo de batería |

### Notas por banco

- **T0:** empieza a 1-5 m en espacio abierto, teléfonos sobre una mesa, pantallas
  encendidas. Repite la matriz a 20 m. En las variantes con casco, conéctalo primero y
  confirma que la ruta dice HFP antes de Iniciar. Anota el modelo en la etiqueta de variante.
- **T0b:** separa A y C (distancia o lista negra `LAB_CTL`) para que el único camino sea
  B. El controlador (A) inunda MESH_PROBE a 5 Hz; B y C registran la traza.
- **T0c:** el concentrador H lleva el casco HFP. Añade enlaces de uno en uno; un
  acompañante marca cualquier corte audible — `mesh_rt_stats` cuenta los subdesbordamientos solo.
- **T1:** ambos vehículos corren la misma variante. Marca cambios de montaje y paradas.
  Solo sirve con consentimiento de GPS en los dos teléfonos (la distancia sale del
  LAB_GPS del par a 1 Hz).
- **T2:** bloquea el teléfono y sigue el guion en papel: casco fuera/dentro, llamada
  entrante, Siri, app de música, aviso de navegación, Modo de bajo consumo, Bluetooth
  apagado/encendido, aviso hablado (TTS) con el teléfono bloqueado. Cada parada de
  captura debe producir una notificación CaptureLost.
- **T3:** corta el enlace con la lista negra 30 s o alejándote; 5 intentos por variante.
  El análisis lee `linkUp.reconnectS`.
- **T5a:** cuarto en silencio, altavoz del casco contra el micrófono del casco. El núcleo
  reproduce un tren de clics MLS de 256 ms cada 2 s y lo busca en la captura; 50 repeticiones.
- **T6:** empieza con la batería entre 90 % y 30 %, sin cargar salvo que la variante lo
  diga. ≥ 1 h por variante en la Fase 0.

### Notas técnicas

Ver la sección en inglés: indicadores LPROBE, parámetros de `LAB_CTL start`, formato
`labrun/1` y umbrales. `T1.convoyGapM` (50 m) es provisional hasta decidir la separación
típica del convoy (DESIGN, Apéndice E n.º 1).

### Diagnóstico de la rodada (`ridelog/1`)

Registro continuo en cada teléfono (no es un banco). En Ajustes → Diagnóstico se activa o
desactiva (activo por defecto mientras la versión de la app sea menor que 1.0), se
exporta la última rodada o todo como un `.zip` por la hoja de compartir y se puede marcar
un problema con un botón fijo (chasquidos, no oigo, no me oyen, retraso, alguien se
desconectó, otro). No guarda voz, ubicación exacta, nombres ni claves; los archivos solo
salen del teléfono al exportarlos. Para unir los archivos de varios teléfonos:
`node tools/lab-analyze/cli.mjs ride <zip|carpeta|jsonl...>` (detalles en la sección en
inglés).
