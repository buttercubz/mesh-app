# mesh-app: investigación técnica de un intercom de voz en malla multiplataforma (iOS + Android, solo teléfonos)

- **Fecha:** 2026-10-22
- **Estado:** investigación. No hay código implementado. Este documento no es una especificación aprobada.
- **Alcance:** app nueva e independiente, *mesh-app*, que convierte teléfonos iPhone y Android en un intercom de grupo tipo Cardo/Sena para caravanas de motos y coches. Funciona sin cobertura celular, sin internet y sin hardware externo: los teléfonos se hablan directamente por Bluetooth y/o Wi-Fi y retransmiten (malla) cuando un compañero queda fuera del alcance directo. Escenario: 2-15 vehículos, hasta ~120 km/h, separación de 20-500 m, casco Bluetooth emparejado al teléfono de cada piloto.
- **Base de evidencia:** cinco informes de investigación (transportes, motor de voz, políticas del SO, malla/producto/seguridad, framework), cada uno contrastado con una verificación adversarial. Cuando la verificación contradice al informe, **prevalece la verificación**. Además se mapeó el repositorio hermano `comvoy-app` (convLink).

**Leyenda de confianza** (se usa en todo el documento):

| Marca | Significado |
|---|---|
| **[H]** | Hecho respaldado por fuente primaria (documentación oficial, código fuente, RFC) y, en la mayoría de casos, re-verificado. |
| **[I]** | Inferencia o cálculo propio a partir de hechos. Es razonable, pero no está medido. |
| **[NV]** | No verificado, o con evidencia solo anecdótica o de terceros. Debe medirse en hardware real. |

Las referencias `[n]` apuntan a la sección 12. Las referencias `[Ln]` apuntan a archivos locales de `comvoy-app`.

---

## 1. Resumen ejecutivo y veredicto de viabilidad

### 1.1 Veredicto

**Sí es viable** un intercom de grupo multiplataforma solo con teléfonos, siempre que se acepte como **intercom de grupo compacto con PTT/VOX y tope de hablantes simultáneos**. **No es viable** igualar a Cardo/Sena en alcance, número de hablantes simultáneos ni experiencia "siempre abierto". Las razones son de física de radio y de política de los sistemas operativos, no de esfuerzo de ingeniería. Ningún dato encontrado permite prometer cobertura de 500 m entre vehículos con teléfonos.

Lo que se puede afirmar con la evidencia disponible a octubre de 2026:

1. **La única radio directa probada entre iPhone y Android es BLE** [H] [1][8]. Wi-Fi Aware funciona iPhone↔iPhone (iOS 26+, iPhone 12+, con emparejamiento por PIN) y Android↔Android. Entre iPhone y Android **no existe ningún data path público confirmado** [H] [48][49][50][51][52]. Google tiene código activo para ello en Nearby (sep-oct 2026), pero no hay lista de dispositivos ni API para terceros [H/NV] [5][6]. La otra vía mixta que funciona hoy es un **hotspot alojado por un Android** al que se unen los iPhone. Es una topología en estrella, no una malla [H] [1][4][58].
2. **Capacidad en BLE.** Un enlace BLE entre teléfonos transporta, de forma realista, **1-2 flujos de voz simultáneos** [I]. bitchat estima ~120 kbps útiles por enlace y ~42 kbps en el aire por flujo de voz de 16 kbps [H, estimación del autor] [183]. Por eso, en BLE, el modo abierto para todos (open-mic) es inviable. El diseño se basa en **PTT + VOX con un tope de 1-2 hablantes**, y hasta ~4 en enlaces Wi-Fi [I].
3. **Alcance.** No hay ninguna medición publicada de teléfono a teléfono entre vehículos en movimiento [NV]. Las referencias informales dan BLE ≈10-30 m "típico" y Wi-Fi Aware ≈35-150 m (prueba en oficina y balcón) [NV] [53]. Cardo anuncia 1,6 km por salto y Sena 2 km [H] [160][167]. Con 20-500 m de separación, **el grupo se partirá en islas** salvo que se circule compacto. La retransmisión multi-salto y la gestión de islas son obligatorias [I].
4. **Usuarios.** Llegar a 15 miembros en la lista es factible, porque el ancho de banda depende de los **hablantes simultáneos** y no de los miembros [I]. Por radio, cada nodo debe mantener solo **3-6 vecinos BLE**. iOS no publica su límite de conexiones; Android garantiza ≥8 canales GATT según CDD; bitchat se limita a 6 enlaces centrales [H] [32][33][178].
5. **Latencia boca-oído estimada** [I]: ~160 ms con 1 salto Wi-Fi Aware, ~215 ms con 2 saltos BLE y ~260 ms con 3 saltos BLE (detalle en §3.3). Los tramos HFP del casco no están medidos para Cardo/Sena y son la mayor incertidumbre. El objetivo de aceptación es MCPTT KPI3: **<300 ms en el p95** [H] [91]. Como referencia real, bitchat declara ~0,5 s a 1 salto con su diseño (AAC, tramas de 64 ms, buffer de 350 ms) [H, dato del autor] [183].
6. **Batería.** No se encontraron mediciones primarias. La estimación es de **baja confianza**: ~2-5 %/h con pantalla apagada y ~6-12 %/h con navegación en pantalla [I/NV] [157]. En mensajería activa, Wi-Fi Aware consumió ~4,25× la energía de BLE en un Pixel 7 [NV] [53].
7. **Segundo plano (background).**
   - En iOS es viable si la sesión de audio arranca en primer plano y **no se detiene nunca durante la ruta**. Es frágil ante interrupciones y, sobre todo, ante **cambios de ruta de audio** (el casco reconectando) [H/NV] [121][123].
   - En Android requiere un *foreground service* `microphone|connectedDevice` iniciado desde la UI visible. Android 17 endurece el audio en segundo plano [H] [140][141][143].

### 1.2 Límites reales esperados frente a Cardo/Sena

| Dimensión | Cardo DMC / Sena Mesh | mesh-app (expectativa realista) | Confianza |
|---|---|---|---|
| Alcance por salto | 1,6 km (Cardo), 2 km (Sena 60S) [160][167] | Decenas de metros (BLE) a ~100+ m (Wi-Fi). No hay medición entre vehículos | [NV] |
| Alcance de grupo | 8 km encadenando [160] | Solo si la separación real es menor que el alcance por salto | [I] |
| Miembros | 31 (Cardo Mesh Boost), 24 (Sena Group Mesh) [162][167] | 2-15 en v1 | [I] |
| Hablantes simultáneos | Cardo 1ª gen. 4 → 6; 2ª gen./Mesh Boost: desconocido [164][165] | 1-2 en BLE; ~4 en Wi-Fi | [I] |
| Modo | Full-duplex abierto con tope de hablantes y mute | PTT + VOX con tope; open-mic solo en grupos pequeños sobre Wi-Fi | [I] |
| Re-unión tras caída | Automática, sin botones [160] | Automática, pero con huecos de segundos (reconexión BLE de "varios segundos") [174] | [I] |
| Latencia | No publicada | ~160-260 ms estimada (1-3 saltos); bitchat ~0,5 s | [I] |
| Autonomía | 13-17 h de conversación [167] | Depende del teléfono; requiere cargador en el vehículo | [I] |
| Interoperabilidad entre marcas | DMC solo Cardo; Mesh 2.0 y 3.0 incompatibles [160][170] | **Ventaja:** cualquier casco, iOS y Android | [H] |
| Mapa de posiciones | No | **Diferenciador** (los teléfonos tienen GPS) | [I] |

### 1.3 Recomendación en una frase por eje

- **Producto:** posicionar la app como "intercom de grupo compacto, solo teléfonos, PTT/VOX, con mapa", no como "reemplazo de Cardo".
- **Arquitectura:**
  - Núcleo compartido en **Rust sans-IO**: protocolo, malla, cripto, Opus 1.6.1, jitter buffer, mezclador, VAD.
  - **Capa nativa** Swift/Kotlin: audio, BLE central y periférico + L2CAP, Wi-Fi Aware, servicio de primer plano.
  - **UI fina con Capacitor** (React 19), con Expo/React Native como plan B.
- **Transporte:** BLE L2CAP como base común y "subida de ancho de banda" a Wi-Fi Aware donde exista (iPhone↔iPhone emparejados, Android↔Android). Hotspot Android opcional para grupos mixtos.
- **Antes de escribir producto:** una **fase 0 de bancos de prueba en hardware real** (§9). Los resultados de alcance entre vehículos y de background en iOS con casco pueden invalidar el concepto.

---

## 2. Matriz de interoperabilidad de transportes

### 2.1 Matriz principal (estado a octubre de 2026)

| Transporte | iOS↔iOS | Android↔Android | iOS↔Android | Throughput (capa app) | Alcance teléfono↔teléfono | Latencia/salto | Setup y diálogos | Background | Máx. pares | Fuentes |
|---|---|---|---|---|---|---|---|---|---|---|
| **BLE GATT / L2CAP CoC (PHY 1M/2M)** | Sí | Sí | **Sí** (recomendado: iPhone central → Android periférico) | **Teléfono↔teléfono no medido en 2025-26.** Ver nota 1 | 10-30 m "típico" en un informe informal. Una prueba urbana dio <10 m. Sin datos entre vehículos [NV] | ~15-50 ms [I]. Intervalo elegido por iOS; Android HIGH = 11,25-15 ms | Solo permiso Bluetooth. Sin diálogo de emparejamiento **si** se evitan atributos cifrados (L2CAP inseguro + cripto propia) [I] | iOS: modos `bluetooth-central/peripheral` (+audio); anuncio en background va al *overflow area*. Android: FGS | iOS: no publicado (`connectionLimitReached`). Android: CDD ≥8, máx. estático 16. Diseñar para 3-6 | [1][12][13][14][16][32][33][183] |
| **BLE LE Coded PHY (S2/S8)** | **No** | Sí, si `isLeCodedPhySupported()` | **No** | 500/125 kbps brutos; mucho menos útil | Hasta ~4× la PHY 1M (dato del fabricante) [NV] | Mayor tiempo en aire | `setPreferredPhy` (API 26) | Solo Android | — | [21][22][23] |
| **Wi-Fi Aware (NAN)** | **Sí** (iOS 26+, iPhone 12+) | Sí (API 26+, según hardware/OEM) | **Condicional y sin demostración pública.** Ver nota 2 | iPhone ~160 Mbps asociado a AP, ~2× con Wi-Fi infra apagado. Android 100-250+ Mbps | ~35-150 m en pruebas informales (oficina/balcón). Sin datos entre vehículos [NV] | Modo `realtime`: no publicado | iOS: entitlement + `WiFiAwareServices` + **PIN de 6 dígitos por par** (DeviceDiscoveryUI/AccessorySetupKit). Android: `NEARBY_WIFI_DEVICES`, Wi-Fi y ubicación activos | iOS: fg/bg **solo mientras la app sigue viva por otro mecanismo**; timeouts de publisher/idle. Android: FGS; puede desaparecer en cualquier momento | iOS: `maximumConnectableDevices` en runtime. Android: 2-8 NDP según dispositivo | [34][35][37][40][44][46][52][53][54] |
| **Hotspot alojado por Android** (LocalOnlyHotspot o grupo Wi-Fi Direct "legacy") + clientes | No (iOS no puede alojar) | Sí | **Sí** (Android aloja, iPhone se une) | Decenas de Mbps [I] | ~50-150 m [I/NV] | ~2-10 ms [I] | Android: LOHS con credenciales aleatorias, o SSID/PSK propios en API 36/36.1 (ver nota 3), o grupo `DIRECT-xy` (API 29). iOS: aprobación del join + permiso Red Local + ubicación; multicast requiere entitlement aprobado por Apple | iOS: `joinOnce=false` (con `true` cae a los 15 s en bg). Android: el usuario puede pararlo; falla con tethering | Estrella. Límite de clientes según dispositivo [NV] | [1][4][58][59][60][61][62][63][64] |
| **Personal Hotspot del iPhone** | Manual | n/a | Manual | similar | similar | similar | El usuario lo activa en Ajustes; depende del operador [NV] | — | — | [1] |
| **Wi-Fi Direct (P2P)** | **No** | Sí | **No** (iOS solo puede unirse al AP "legacy" del grupo) | 50+ Mbps [I] | ~50-150 m [I] | baja | Diálogos Android | — | Según dispositivo; en Pixel 9a compite con NAN | [1][10] |
| **AWDL / Multipeer / Network.framework peer-to-peer** | Sí | **No** | **No** | alta | ~similar a Wi-Fi | baja | Permiso Red Local | Solo iOS. **Multipeer obsoleto desde Xcode 27** | 8 pares observados entre 12 iPhones; DTS: "not intended to scale" | [41][55] |
| **Google Nearby Connections** | Parcial | Sí | Parcial y en evolución | media-alta | — | — | SDK Nearby | — | según estrategia | [1][2][3][4] |
| **BT clásico RFCOMM** | Solo MFi | Sí | No (requiere permiso especial de Apple) | ~1-2 Mbps | ~10-30 m | — | emparejamiento | — | 7 (piconet) | [1] |

**Nota 1 — Throughput BLE.** Las cifras corregidas son estas:
- [H] 400-600 kbps es **un solo enlace** periférico→iPhone contra un periférico Nordic optimizado (2M, DLE, MTU 244, 15 ms). No es un agregado [12].
- [H] Android→mismo periférico: 1000-1200 kbps [12].
- [H] iPhone↔iPhone L2CAP en la época de iOS 11: 36-138 kbps [13].
- [H] iPhone central con L2CAP: 36 KB/s, que suben a 60 KB/s con intervalo de 15 ms [14].
- [NV] El dato de 100 KB/s con iPhone periférico se midió contra un ESP32-S3, no contra un teléfono [14].
- [H] Punch Through observó que iOS da a BLE solo ~50 % de un intervalo de 15 ms [16].
- [I] Presupuesto de diseño: **≤ ~120 kbps útiles por enlace**, con objetivo de uso ≤50 %. Menos aún mientras el casco usa HFP.

**Nota 2 — Wi-Fi Aware iPhone↔Android.** Apple exige que el par cumpla Wi-Fi Aware 4.0 con emparejamiento seguro y caché NIK, protección de multicast/gestión/beacons, y **no** soporta IPv6 NDP [H] [17]. Evidencia en contra:
- Galaxy S25 reporta `isAwarePairingSupported()==true` y aun así falla (PASN status 15, "missing DCEA attribute") [H] [49].
- En otro caso el emparejamiento llegó a "authenticated", pero iOS nunca respondió a la petición NDP del Android [H] [50].
- Conclusión: el flag es **necesario pero no suficiente** [H].
- Android 17 traslada la seguridad NAN a `wpa_supplicant` y al framework, y añade `AwareDataPathRequest` (API 37). Es la ruta más probable hacia la compatibilidad, pero se fragmentará por fabricante de chip [H/I] [45][47][66].

**Nota 3 — Hotspot con SSID/PSK fijos.** El informe de transportes da por hecho `startLocalOnlyHotspotWithConfiguration` (API 36) con `setWifiSsid`/`setPassphrase` (36.1) [62][63]. El informe de políticas lo encontró tras un flag de AOSP y no pudo confirmar que esté disponible para apps no-sistema [NV]. Hay que verificarlo en dispositivo.

### 2.2 Controles BLE por SO

| Capacidad | iOS (CoreBluetooth, iOS 26/27) | Android 14-17 |
|---|---|---|
| Central + periférico simultáneos | Sí (lo usan Nearby y bitchat) [2][8] | Sí |
| Intervalo de conexión | La app no lo controla. Guía para accesorios: mín. ≥15 ms y múltiplo de 15; HID 11,25 ms; accesorios espaciales 7,5 ms (ADG R31, 2026-09-21) [17]. Se ha observado a iOS 26 negociar 7,5 ms por su cuenta [18] | `requestConnectionPriority`: HIGH 11,25-15 ms, BALANCED 30-50, LOW_POWER 100-125 [19]. **No hay evidencia de que un Android periférico (servidor GATT) pueda pedir 15 ms al iPhone central** [NV] |
| MTU | Negociado por el sistema | Android 14+ pide 517 en la primera solicitud [19] |
| PHY 2M | Automática, sin API [I] | `setPreferredPhy` [19] |
| Coded PHY | No disponible; Apple "no roadmap or timeline" [21] | Sí, si el hardware lo soporta [20] |
| L2CAP CoC | `publishL2CAPChannel`/`openL2CAPChannel` (iOS 11) | `listenUsingInsecureL2capChannel` (API 29) [20] |
| Anuncio | Foreground: 28 B (UUIDs + nombre) + 10 B de scan response solo para el nombre. Background: sin nombre y UUIDs en *overflow area* [24] | Tipos AD completos; extended advertising donde exista |
| Background | Modos `bluetooth-central/peripheral`; despertares de ~10 s si la app está suspendida [26][28] | FGS `connectedDevice` |
| Límite de conexiones | No publicado; varía por dispositivo y se comparte con otras apps [32] | Propiedad de sistema en runtime; CDD ≥8; estático 16 [33] |
| Novedad iOS 26 | Una app con `CBManager` que inicia una **Live Activity** antes de pasar a background conserva privilegios BT de primer plano (escanear sin UUIDs, sin filtrar duplicados) [H] [31]. **No se sabe** si levanta las restricciones del anuncio (overflow, nombre) [NV] | — |

### 2.3 Descubrimiento con pantallas bloqueadas: la regla de roles

- [H] Con la app en background, iOS no anuncia el nombre local y mueve todos los UUID al *overflow area*, "discoverable only by an iOS device explicitly scanning for them" [24]. Apple dice que una central no-Apple "will not be able to decode the overflow area" y recomienda que la app iOS actúe como **central** [27][28].
- [H, con matiz] El formato del overflow está documentado por ingeniería inversa: manufacturer data `ff 4c 00 01` seguido de un bitmask de 128 bits con un bit por UUID según un hash de Apple. Un Android **sí puede** detectar un iPhone en background probando ese bit. Es probabilístico (colisiones con otras apps), no está documentado y puede romperse en cualquier versión de iOS [29][30].
- [NV, carga crítica] Según un estudio de 2020, el overflow solo se recibía mientras la **pantalla del iPhone receptor estaba encendida** [29]. Si sigue siendo así en iOS 26/27, **dos iPhones bloqueados en el bolsillo podrían no encontrarse nunca**. Los enlaces ya establecidos sí siguen funcionando.
- [I] Un iPhone en background hace escaneo pasivo e ignora `SCAN_RSP` [28]. Por eso el Android debe poner su UUID de servicio de 128 bits en el **ADV_IND primario**, no en la scan response.

**Regla operativa recomendada** [I]:
1. **iPhone↔Android:** el iPhone es central y el Android periférico. El Android anuncia un UUID fijo desde su FGS.
2. **iPhone↔iPhone:** formar el enlace con pantalla encendida (lobby previo a la ruta) y mantenerlo. Usar Wi-Fi Aware emparejado para reconectar. Opcionalmente, usar Android cercanos como puentes.
3. **Android↔Android:** cualquier rol.
4. **No poner datos de malla en anuncios iOS.** Todo va por GATT/L2CAP tras conectar.

### 2.4 Coexistencia de radios en el teléfono

- [H] Pixel 9a: Wi-Fi Aware y Wi-Fi Direct/SoftAP compiten por la interfaz. `createGroup` devuelve BUSY mientras existe `aware_nmi0`. bitchat lo resolvió liberando Aware antes de alojar [10].
- [H] Android: Aware puede no estar disponible con SoftAP, tethering o Wi-Fi Direct activos. Ante `ACTION_WIFI_AWARE_STATE_CHANGED` hay que descartar las sesiones [46].
- [H] iPhone: el throughput de Aware se reduce a la mitad aproximadamente si el teléfono está asociado a un AP [54].
- [I/NV] El casco en HFP (eSCO) reserva ranuras periódicas en la misma radio de 2,4 GHz que usan BLE y el Wi-Fi de 2,4 GHz. Es plausible que reste decenas de por ciento de capacidad BLE, pero **no hay medición publicada**. Conviene preferir 5 GHz para Aware/hotspot (Apple ADG indica que iOS prefiere 5 GHz si el par lo soporta [17]).

---

## 3. Motor de voz

### 3.1 La restricción dominante: el enlace del casco

- [H] Teléfono↔casco usa HFP: **CVSD a 8 kHz (banda estrecha)** o **mSBC a 16 kHz (banda ancha)** en ambos sentidos [67][69].
- [H] HFP 1.9 añade **LC3-SWB**. En Android 17 el "audio-managed SCO" prefiere LC3 HW > LC3 SW > mSBC HW > mSBC SW > CVSD [68][70].
- [H] Android también soporta LE Audio (`TYPE_BLE_HEADSET`) [96].
- [H] iOS no tiene una ruta pública de casco LE Audio [97].
- [H] `bluetoothHighQualityRecording` (iOS 26) solo funciona con AirPods compatibles, solo en modo `default`, no está disponible en la UE, "may increase input latency" y "isn't recommended for real-time communication". **No usar** [72].
- [H] AirPods: cuando el micrófono está activo, la reproducción también baja a 16 kHz [71].
- [H] Sena 50S: "HD Voice" (llamadas en banda ancha) está activo por defecto, pero **se desactiva si se habilita "Bluetooth Intercom Audio Multitasking"** [73]. [NV] No se pudo verificar si Cardo negocia mSBC.
- **Decisión de diseño** [I]: el ancho de banda del codificador sigue a la **ruta de captura del emisor**: CVSD → NB, mSBC → WB, LC3-SWB → SWB posible solo en Android. No se fija en WB. Opus BWE/HD no aporta nada cuando la reproducción va por SCO de 16 kHz [75].

### 3.2 Códec y configuración recomendada

**Biblioteca:** libopus **1.6.1** (última estable, enero de 2026), compilada una sola vez en el núcleo compartido.
- Compilación *float* con `--enable-deep-plc --enable-osce`. El modo *fixed-point* es incompatible con las funciones ML [83].
- DRED compilado pero desactivado por defecto; se habilita por ruta y por negociación (ver abajo).
- **La misma build en iOS y Android.** La versión del códec viaja en el handshake de grupo [H] [75][76][77].

| Parámetro | Valor recomendado | Motivo / evidencia |
|---|---|---|
| Aplicación / señal | `OPUS_APPLICATION_VOIP`, `OPUS_SIGNAL_VOICE` | Camino SILK de voz [85] |
| Ancho de banda | API a 16 kHz. `MAX_BANDWIDTH` = WB, o NB si la captura es CVSD a 8 kHz | Límite del HFP [67] |
| Trama | 20 ms por defecto; 40 ms en rutas BLE o congestionadas. Nunca 10 ms (penalización de ~3 kbps y sin OSCE) ni 60 ms | [H] OSCE/NoLACE también funciona con paquetes de 40/60 ms (SILK los codifica como 2-3 tramas de 20 ms). Solo se salta 10 ms, NB y MB [81] |
| Complejidad del codificador | 8-10 (medir batería) | — |
| Complejidad del decodificador | 7 (Deep PLC + NoLACE). Coste de CPU ~1 % de un núcleo de portátil (dato del proveedor) | [74] |
| VBR | Restringido (tamaño de paquete acotado) | Tiempo de aire predecible |
| DTX | Activado, pero **la transmisión la decide un VAD externo** | Ver abajo |
| Pista de pérdida | `OPUS_SET_PACKET_LOSS_PERC` = pérdida suavizada del peor vecino, limitada a 5-25 % | Con pista 0 no hay FEC; la pista **cambia el ancho de banda de audio** [78] |
| ML de supresión de ruido | Desactivada por defecto. RNNoise (10 ms) es opcional y re-entrenable con viento. DeepFilterNet no (~40 ms de latencia) | [112][113] |

**FEC in-band (LBRR) — tabla corregida y medida** [H]. Medida por el verificador con libopus 1.6.1 (VBR, complejidad 10, 20 ms, `MAX_BANDWIDTH`=WB) [78][79]:

| Bitrate | Pérdida 3 % | 5 % | 10 % | 20 % | 30 % |
|---|---|---|---|---|---|
| 16 kbps | sin FEC | sin FEC | sin FEC | sin FEC | MB + LBRR |
| 20 kbps | sin FEC | sin FEC | MB + LBRR | WB + LBRR | WB + LBRR |
| 24 kbps | WB + LBRR | WB + LBRR | WB + LBRR | WB + LBRR | WB + LBRR |

- Los bits de FEC salen **del mismo presupuesto**. A 24 kbps de objetivo, la media real fue ~20,7 kbps.
- **Regla:** para tener WB con FEC hacen falta **≥24 kbps**.
- LBRR solo protege la trama inmediatamente anterior [85]. Recuperar cuesta **+1 trama de jitter buffer**: +20 ms con tramas de 20 ms y **+40 ms con paquetes de 40 ms** [H].

**DRED (redundancia neuronal profunda)** [H/I]:
- Los bits de DRED también salen del presupuesto.
- Con tramas de 20 ms es caro: 0 kbps de DRED a 16 kbps; ~7,8 kbps para ~80 ms a 24 kbps.
- Con **paquetes de 40 ms es barato** (resultado de fórmula, no de escucha):
  - ~3,9 kbps cubren 280 ms a 20 kbps con 10 % de pérdida.
  - ~5,8 kbps cubren 360 ms a 24 kbps con 10 %.
  - ~7,2 kbps cubren 720 ms a 24 kbps con 20 %.
- **LBRR y DRED compiten** por los bits (con LBRR activo, DRED ≈0 por debajo de 48 kbps). Hay que **elegir uno por ruta**.
- DRED solo rellena huecos más cortos que la holgura del jitter buffer.
- El formato está versionado: `DRED_EXPERIMENTAL_VERSION` es 12 en 1.6/1.6.1 y 10 en 1.5.2. El borrador IETF -07 (agosto de 2026) está en WG Last Call y **cambiará el ID de extensión**. Si hay discrepancia, se ignora sin ruido. Hay que negociarlo en el handshake [82][84].

**Política de bitrate propuesta** (a validar con `opus_demo` y en campo) [I]:

| Ruta | Configuración |
|---|---|
| Wi-Fi limpia | 16-20 kbps WB, sin FEC, 20 ms |
| Wi-Fi con pérdidas ≥3 % | 24 kbps WB + LBRR, 20 ms |
| BLE, opción A | 16 kbps WB, 40 ms, sin FEC. Se apoya en duplicados multi-ruta (deduplicados por `(src, seq)`) y en Deep PLC |
| BLE, opción B | 20-24 kbps, 40 ms, **DRED sin LBRR** (si todos los pares lo soportan) |
| BLE degradado | 12-14 kbps NB, 40 ms |

**VAD/DTX** [H]:
- El DTX de Opus no sirve como puerta de voz con viento o motor. Se activa a los 200 ms. Una trama cuenta como activa si la probabilidad de actividad es ≥0,1 **o** si está a menos de 25 dB del pico de voz. El análisis de tonalidad solo corre con complejidad ≥7 [80].
- Sumar N micrófonos abiertos eleva el ruido de fondo ~10·log10(N) dB (+6 dB con N=4) [I].
- **VAD externo recomendado:** Silero VAD. Funciona a 8/16 kHz con bloques de 32 ms, tarda <1 ms por bloque, pesa ~2 MB y es MIT [114].
- Ajustes: ~300 ms de *hangover* y 1 bloque de pre-roll que el receptor absorbe con *accelerate*.
- [NV] No hay comparación de VADs con viento de casco a 100+ km/h.

**Códecs descartados** [H]:
- Lyra V2: sin iOS e inactivo desde 2024 [87].
- Codec2: solo el modo 3200 está mantenido [88].
- EnCodec: implementación de referencia en PyTorch [89].
- En BLE el cuello de botella son los paquetes por evento de conexión y la pérdida, no los bytes [I].

### 3.3 Presupuesto de latencia (boca-oído, una vía; estimación [I])

| Etapa | 1 salto Wi-Fi Aware | 2 saltos BLE | 3 saltos BLE | Base / confianza |
|---|---|---|---|---|
| Micro del casco + DSP + eSCO de subida | 25 (15-40) | 25 | 25 | Patente de Bose ~40 ms [92]. Sin datos de Cardo/Sena. **Baja** |
| Buffer de captura del SO + VPIO/APM | 15 (10-25) | 15 | 15 | WebRTC iOS: buffer de 20 ms [103]. Media |
| Tramado + look-ahead de Opus (20 ms) | 26,5 | 26,5 | 26,5 | RFC 6716 / Lyra [85][87]. El mapeo exacto de 26,5 ms a la trama de 20 ms es inferencia |
| Codificar + empaquetar | 1 | 1 | 1 | [I] |
| Radio por salto | 5 (3-10) | 2×20 | 3×20 | [I] Medio intervalo + reintentos |
| Reenvío / colas en relés | 0 | 3 | 2×3 | [I] |
| Jitter buffer (incluye +1 trama por FEC) | 40 (20-60) | 60 | 80 | [I], tipo NetEq [115] |
| Decodificar + PLC + mezcla (tick de 10 ms) + limitador | 5 | 5 | 5 | [I] |
| Buffer de reproducción del SO | 15 | 15 | 15 | [I] |
| eSCO de bajada + DSP del casco | 25 (15-40) | 25 | 25 | **Baja** |
| **Total** | **~158 (104-244)** | **~216** | **~259** | G.114: <150 ms "esencialmente transparente", 400 ms límite de planificación [90]. MCPTT KPI3: <300 ms en el p95 [91] |
| Extra con paquetes de 40 ms | +20 (tramado) | +20 | +20 | Más otros +20 si se usa LBRR con 40 ms |

**Consecuencias** [I]:
- Limitar la retransmisión de **voz a 3 saltos**.
- La experiencia full-duplex solo es razonable a 1-2 saltos. Con 3 saltos se comporta como una radio PTT.
- El dato decisivo es la latencia real de los dos tramos HFP de cada modelo de casco. Ninguna fuente primaria la da para Cardo/Sena.
- Correcciones a cifras citadas a menudo [H]: la cifra de "20-30 ms" de Silicon Labs no se pudo verificar (HTTP 403). El hilo 679274 de Apple citado como "193-260 ms" describe en realidad AirPods **en A2DP** con `outputLatency` estable de 160,667 ms y una latencia medida que bajó de ~215-220 ms a ~155-160 ms [95].
- [H] Abrir SCO "puede tardar varios segundos" y el callback de `setCommunicationDevice` puede tardar hasta 30 s [67][96]. **Abrir SCO por cada pulsación de PTT no puede cumplir el KPI1 de 300 ms de acceso.** La ruta de voz del casco se abre una vez y se mantiene toda la ruta.

### 3.4 Eco y ruido

- **iOS** [H]: VPIO (`setVoiceProcessingEnabled`) aplica AEC/AGC. La mezcla final debe pasar por el nodo de salida VP para que sea la referencia del AEC [99]. iOS 18 puede ampliar el buffer de E/S a ~23 ms en silencio si "Sound Recognition" o "Vocal Shortcuts" están activos [104].
- **Android** [H]:
  - Preset `VOICE_COMMUNICATION`. Comprobar `AcousticEchoCanceler.getEnabled()` [108]; muchos dispositivos tienen AEC por hardware que no reportan [109].
  - Fallback: WebRTC APM empaquetado (AEC3 + NS + AGC2). AECM se está retirando en favor de AEC3 [110][111].
  - Alternativa en Rust: `sonora` 0.2.0, un port de AEC3/NS/AGC2, joven y sin soporte móvil documentado [239].
- **HFP** [H]: el casco puede enviar `AT+NREC=0` y asumir la cancelación de eco y ruido.
- [I] Apilar tres supresores de ruido (casco, SO y ML) produce artefactos. La ML va desactivada por defecto.
- [I] Con un recorrido de ida y vuelta en malla >300 ms, cualquier eco residual se nota mucho. **Cada nodo necesita AEC sobre su mezcla final.**

### 3.5 Cascos Bluetooth y enrutamiento de audio

**Configuración de sesión en iOS:**
- `.playAndRecord`, modo `.voiceChat` (implica `.allowBluetoothHFP`), VP activado, buffer de E/S de 10-20 ms, volver a leer los formatos tras activar VP [H] [99][100].
- Declarar `UIBackgroundModes: audio`.
- Ducking de otras apps con `voiceProcessingOtherAudioDuckingConfiguration`, nivel mínimo (iOS 17+) [102].
- **Conflicto sin resolver: `.mixWithOthers`.**
  - El informe de voz la recomienda para convivir con apps de navegación [101].
  - El de políticas la desaconseja: DTS explicó que las sesiones de grabación *mixables* son un camino heredado que **no se activa en background** [128].
  - bitchat usa `.mixWithOthers` y solo reproduce en primer plano [138][179].
  - **Recomendación** [I]: sesión **no mixable** por defecto, para no perder la reactivación en background. Validar en campo que Google Maps/Waze (que usan sus propias sesiones mixables con *duck*) se siguen oyendo.

**Configuración en Android:**
- `MODE_IN_COMMUNICATION` + `setCommunicationDevice` (API 31; `startBluetoothSco` está obsoleto desde la API 34), prefiriendo `TYPE_BLE_HEADSET` sobre `TYPE_BLUETOOTH_SCO` [H] [67][96].
- Oboe con `Usage::VoiceCommunication` + `InputPreset::VoiceCommunication` + `LowLatency` [106][107].
- `clearCommunicationDevice()` al salir.
- **Alternativa fuerte: Jetpack Core-Telecom** (`androidx.core:core-telecom` 1.0.0) [H] [158].
  - Modela la ruta como una llamada propia: `MANAGE_OWN_CALLS`, FGS `phoneCall` y notificación `CallStyle` en ≤5 s.
  - Telecom cambia los endpoints, incluido el casco, vía `CallControlScope`. Google pide **no** llamar a `setCommunicationDevice` en ese caso.
  - Un FGS delegado por Telecom obtiene *while-in-use* bajo las reglas de Android 17 [I].

**Preferencias y efectos secundarios** [H]:
- La app de teléfono **siempre tiene prioridad** sobre SCO. Una llamada real se lleva el enlace y la documentación heredada dice que **no se devuelve automáticamente** [67].
- La última app que pone `MODE_IN_COMMUNICATION` gana el ruteo [67].
- iOS interrumpe la sesión con las llamadas.
- **Música:** mientras se mantiene SCO, la música de otras apps pierde A2DP estéreo. En iOS, HFP tiene prioridad de ruteo. En Android, A2DP se suspende o bloquea, y en Android 17 el SCO gestionado impide un patch A2DP simultáneo [68][100].
- Los avisos de navegación suenan en mono por HFP y con ducking.
- **Cardo DMC:** una llamada o un GPS por HFP interrumpe el intercom propio del casco [165].
- Aceptación por parte de los pilotos: desconocida [NV].

**Botones del casco como PTT:**
- iOS: el framework PushToTalk (iOS 17) tiene `setAccessoryButtonEventsEnabled` para mapear botones de accesorio a inicio/fin de transmisión [H] [127].
- Android 17: "media key events from an external device" otorgan *while-in-use* a un FGS [H] [143].
- [NV] Ninguna de las dos vías está probada con Cardo/Sena.

### 3.6 Mezcla, jitter y modos (open mic / VOX / PTT)

- **Jitter buffer** [H/I]: uno adaptativo por **originador**, estilo NetEq [115]:
  - Ticks de 10 ms y objetivo según el p95 del jitter de llegada.
  - Mínimo de 20 ms (+1 trama con FEC) y máximo de 200 ms.
  - *accelerate* / *preemptive expand*.
  - Deduplicar por `(src, seq)` y usar la llegada de la primera copia.
  - Ni iOS ni Android traen un jitter buffer de Opus con FEC/PLC/time-stretch, así que hay que escribirlo o portarlo (por ejemplo NetEq) [H].
- **Mezclador** [H/I]: decodificar → AGC lenta + puerta de ruido por flujo → sumar **≤3-4 hablantes** (Cardo DMC 1ª gen. "the first 4 that speak" [165]; WebRTC antiguo mezclaba los 3 más fuertes [116]) → limitador tipo AGC2. Los TTS de la app ("Ana se unió", "canal ocupado") pasan por el mismo mezclador para quedar dentro de la referencia del AEC.
- **Modos** [I]:

| Modo | Uso | Comentario |
|---|---|---|
| **PTT** | Grupos grandes, solo BLE o ruido extremo | Botón en pantalla, botón de manillar BLE (iOS PTT permite iniciar tras un cambio de característica CoreBluetooth [126]) o botón del casco (NV) |
| **VOX** (VAD + tope de hablantes) | Opción natural de manos libres en moto | El tope lo imponen los relés (§5.4) |
| **Open-mic** | Solo grupos pequeños (2-4) sobre enlaces Wi-Fi | Inviable en BLE para todos los miembros |

El modo por defecto es una decisión de producto (§11).

---

## 4. Políticas del SO: background, permisos, entitlements, revisión, batería

### 4.1 iOS: mantener vivo el proceso

- [H] Las apps se suspenden en background salvo con modos declarados [118]. Con la app suspendida, los sockets locales "just won't work" y ningún paquete local la despierta (DTS) [119][120].
- [H] **Ancla principal:** sesión `playAndRecord` activa con `UIBackgroundModes: audio`. La documentación de la categoría `.record` dice que para seguir grabando con la pantalla bloqueada hay que añadir `audio` a `UIBackgroundModes` [121].
- [H] **Segunda ancla legítima:** el modo `location` con actualizaciones continuas (`CLBackgroundActivitySession`, iOS 17+). El sistema "won't suspend" la app mientras la localización está activa. Encaja con un mapa de posiciones del convoy, pero **no permite arrancar el micrófono desde background** [122]. convLink ya usa este modo.
- [H] **Wi-Fi Aware no concede tiempo de ejecución propio.** Hay que mantener la app viva por otro mecanismo [34][36].
- [H] Con la app suspendida, cada evento BLE da ~10 s de ejecución [26][28].
- [H] TN3115: no hay relanzamiento tras *force-quit*. Desde iOS 26, varios casos de relanzamiento **solo aplican a apps que configuran accesorios con AccessorySetupKit** [134]. Conviene indicar a los usuarios que no cierren la app deslizándola.

### 4.2 iOS: arrancar y re-arrancar el micrófono

- [H] **No se puede *iniciar* la grabación desde background.** El error es `!rec` / `cannotStartRecording` [123].
- [H, matiz] Desde iOS 13.1 hay reportes de que la grabación que **ya estaba activa** antes de una interrupción (Siri, llamada) sí se puede reanudar en background [123].
- [NV, carga crítica] Los mismos reportes (2019) dicen que un **cambio de ruta** con la app en background (p. ej. el casco BT reconectándose) detenía la captura y **no se recuperaba**. Los cascos Cardo/Sena caen y reconectan HFP con frecuencia. **Hay que probarlo en iOS 26/27** (§9).
- [H] iOS 27 deja obsoleta `interruptionNotification` en favor de `didBecomeInactive` / `resumptionRecommendation`, que no se atascan si nunca llega el evento de fin [124]. `setPrefersNoInterruptionsFromSystemAlerts` (iOS 14.5+) evita que el banner de llamada entrante interrumpa la sesión salvo que la llamada se acepte [125].
- **Diseño** [I]:
  - Arrancar el motor en primer plano (botón "Unirse") y **no pararlo nunca durante la ruta**. VAD, PTT y DTX filtran *después* de capturar.
  - Si la captura muere: notificación local "Toca para reanudar el intercom" + Live Activity.

### 4.3 iOS: alternativas de sistema

| Mecanismo | Lo que da | Lo que no da | Estado |
|---|---|---|---|
| **PushToTalk** (iOS 16+) | UI de sistema, grabación en background al activarse, botones de accesorio (iOS 17), CarPlay | Despierta una app suspendida solo por APNs o Local Push Connectivity (LPC). **Re-suspende la app tras cada transmisión.** Solo un canal PTT activo en todo el sistema; unirse solo en primer plano | [H] [126][127][128]. DTS recomienda enviar push "every time" [128]. LPC no es solo Wi-Fi (SSID, LTE privada, Ethernet iOS 26, MCX iOS 27), pero requiere entitlement `app-push-provider` y un servidor en esa red [129]: **no sirve para una malla offline** |
| **CallKit / LiveCommunicationKit** (iOS 17.4+) | Sesión de audio con prioridad de llamada, gestionada por el sistema; llamada saliente sin push | [NV] Si App Review acepta un intercom modelado como llamada; si una llamada de horas sin push mantiene viva la app; disponibilidad regional | [H] Bug intermitente de `didActivate`: DTS recomienda `setConfiguration` antes de cada llamada; corrección parcial en iOS 27 beta 5 [130][131][132][133]. **Prototipar** |

### 4.4 iOS: transportes según el estado de la app

| Transporte | Primer plano | Background con app viva (audio) | Suspendida |
|---|---|---|---|
| Enlaces BLE existentes | Completo | Funcionan | El sistema mantiene el enlace; ~10 s por evento |
| Escaneo BLE | Libre | Hay que filtrar por UUID; duplicados agrupados; intervalo mayor [25][26] | Igual, entregado al despertar |
| Anuncio BLE | 28 B | Sin nombre; UUIDs en overflow; "may slow down… stop altogether" [28] | Igual |
| Wi-Fi Aware | Emparejamiento + conexión | Conexión a pares **ya emparejados** [34] | No |
| Cliente de hotspot | Sí | Sí con `joinOnce=false` [59] | Los sockets mueren |
| Captura de micrófono | Se puede iniciar | Continúa si no se detiene; **no se puede (re)iniciar** | No |

### 4.5 iOS: red local, hotspot, multicast

- [H] TN3179: enviar UDP, abrir TCP saliente o recibir multicast/broadcast requiere el permiso de **Red Local**. Si el permiso está "undetermined" y la app está en background, la operación **se deniega sin mostrar diálogo**. Hay que provocar el diálogo en el onboarding en primer plano [60].
- [H] Multicast/broadcast requiere `com.apple.developer.networking.multicast`, que **pide aprobación de Apple** [61]. Alternativa: fan-out unicast.
- [H] `NEHotspotConfiguration`: muestra aprobación del sistema; usar `joinOnce=false` [58][59].
- [NV] No se pudo verificar si iOS se mantiene asociado indefinidamente a un hotspot sin internet mientras envía su tráfico de internet por celular.

### 4.6 Android: servicio de primer plano y Android 17

- [H] **FGS `microphone|connectedDevice`** (opcional `phoneCall` con Telecom; `location` si se comparte GPS) [140]:
  - `microphone` requiere `FOREGROUND_SERVICE_MICROPHONE` + `RECORD_AUDIO` concedido.
  - **Desde target 34+, crear un FGS de micrófono desde background lanza `SecurityException`.** Debe iniciarse con una actividad visible o con una exención (interacción con notificación o widget, etc.) [141].
  - `connectedDevice` acepta `BLUETOOTH_CONNECT/ADVERTISE/SCAN` o `CHANGE_WIFI_STATE`, entre otros.
  - Ninguno de los dos tipos tiene límite de tiempo (solo `mediaProcessing` lo tiene).
- [H] **Endurecimiento del audio en background en Android 17**, para **todas** las apps que corren en Android 17, sea cual sea su target [143]:
  - Reproducción, foco y volumen fallan **en silencio** sin una actividad visible o un FGS que no sea `shortService`. `requestAudioFocus` devuelve `AUDIOFOCUS_REQUEST_FAILED`.
  - Con target 37, el FGS además necesita capacidad *while-in-use* (WIU): iniciado mientras la app era visible, por intención del usuario (clic en notificación o widget, **teclas multimedia de un dispositivo externo**) o por delegación del sistema.
  - Prueba: `adb shell cmd audio set-enable-hardening throw`. `dumpsys` muestra `AudioHardening` (`level: full` = FGS sin WIU; `level: partial` = sin FGS).
- [I] **Reinicio tras la muerte del proceso:** relanzar desde background un FGS con tipo `microphone` lanza `SecurityException` (no falla en silencio). Un reinicio solo con `connectedDevice` arranca, pero sin WIU, y con target 37 su reproducción queda muda sin error. **Reiniciar solo mediante una acción de notificación o un botón del casco.**
- [H] **`ACCESS_LOCAL_NETWORK`** (obligatorio con target 37):
  - Bloquea todo TCP/UDP de LAN, entrante y saliente, unicast/multicast/broadcast.
  - Los fallos aparecen como timeout (TCP) o EPERM (UDP).
  - Está en el grupo `NEARBY_DEVICES`, así que no se vuelve a pedir si ya se concedió Bluetooth.
  - Cubre link-local IPv6 en interfaces Wi-Fi [146][147].
  - [I] Probablemente aplica a sockets sobre el NDP de Wi-Fi Aware. **BLE no se ve afectado.**
- [H] Calendario de Play: target 36 obligatorio desde el 31-08-2026 (prórroga al 01-11-2026). Las reglas de target 37 se pueden diferir ~1 año, pero las reglas de audio "all apps" ya aplican en dispositivos con Android 17 [148].
- [H] Android 17 añade límites de memoria por app. Las muertes aparecen como `MemoryLimiter:AnonSwap`. Hay que mantener pequeños los buffers de jitter y las cachés [144].

### 4.7 Android: política de escaneo BLE (leída en el código de AOSP; no re-verificada)

| Modo de escaneo | Ventana/intervalo | Ciclo útil | Con pantalla apagada (con FGS) |
|---|---|---|---|
| LOW_LATENCY | 100/100 ms | 100 % | Se mantiene LOW_LATENCY |
| BALANCED | 183/730 ms | ~25 % | SCREEN_OFF_BALANCED 183/730 ms |
| LOW_POWER | 140/1400 ms | 10 % | SCREEN_OFF 512/10240 ms (5 %) |
| Tras 10 min (no exento) | filtrados → tope LOW_POWER; sin filtro → oportunista | — | Exentos: OPPORTUNISTIC y `CALLBACK_TYPE_FIRST_MATCH` |

Fuente: [153][154]. Otros puntos:
- Cuota de **5 inicios de escaneo cada 30 s**.
- Los escaneos sin filtro se detienen con la pantalla apagada.
- Con perfiles BT conectados, el escaneo tiene tope BALANCED.

**Recomendación** [I]:
- Usar siempre `ScanFilter` por UUID con `neverForLocation`.
- Re-arrancar los escaneos filtrados cada ~9 min respetando la cuota.
- Anunciar a 250 ms mientras se forma el grupo y a 1 s una vez estable.

### 4.8 Android: Doze, OEMs y exenciones

- [H] Doze solo se activa "unplugged and stationary… screen off", algo raro en un vehículo en marcha. Un proceso con FGS conserva la red durante Doze y en ahorro de batería [150][151].
- [H] El verdadero riesgo son los gestores de batería de los OEM. Samsung, Xiaomi, Huawei y OnePlus puntúan 5/5 en dontkillmyapp [152].
- [H] Play permite `REQUEST_IGNORE_BATTERY_OPTIMIZATIONS` a apps de llamadas que no pueden usar FCM porque Doze rompe su función principal. Un intercom offline encaja por inferencia [149].
- [H] El usuario puede detener el FGS desde el Task Manager de Android 13+.

### 4.9 Permisos y entitlements

| Ítem | iOS (26/27) | Android (12-17) |
|---|---|---|
| Ancla de background | `UIBackgroundModes`: `audio`, `bluetooth-central`, `bluetooth-peripheral` (+ `location` si hay mapa). `voip` solo con CallKit; `push-to-talk` solo con el framework PTT | FGS `microphone` + `connectedDevice` (+ `phoneCall` con `MANAGE_OWN_CALLS` si se usa Telecom; + `location`) iniciado desde la UI |
| Micrófono | `NSMicrophoneUsageDescription` | `RECORD_AUDIO` (en tiempo de ejecución, *while-in-use*) |
| Bluetooth | `NSBluetoothAlwaysUsageDescription` | `BLUETOOTH_SCAN` (`neverForLocation`), `BLUETOOTH_ADVERTISE`, `BLUETOOTH_CONNECT` (31+). Los heredados con `maxSdkVersion 30`. Dar soporte a API 29-30 obliga a `ACCESS_BACKGROUND_LOCATION` [155] |
| Wi-Fi Aware | Entitlement `com.apple.developer.wifi-aware` [Publish, Subscribe]; `WiFiAwareServices` (nombre ≤15 caracteres, `_name._udp/_tcp`; **un nombre inválido hace crashear la app**) [35] | `NEARBY_WIFI_DEVICES` (`neverForLocation`, 33+), `ACCESS_FINE_LOCATION` (`maxSdk 32`), `ACCESS/CHANGE_WIFI_STATE`, `CHANGE_NETWORK_STATE` [156] |
| Hotspot | `NEHotspotConfiguration` + entitlement Hotspot Configuration | `CHANGE_WIFI_STATE` + `NEARBY_WIFI_DEVICES`; el llamante debe ser visible o tener FGS |
| Red local | `NSLocalNetworkUsageDescription` (+`NSBonjourServices` si se usa Bonjour); multicast con entitlement aprobado por Apple | `ACCESS_LOCAL_NETWORK` (target 37+) |
| Batería / OEM | — | `REQUEST_IGNORE_BATTERY_OPTIMIZATIONS` (justificado), `WAKE_LOCK`, `POST_NOTIFICATIONS` (33+) |
| Declaraciones de tienda | Notas de revisión + vídeo del audio en background; etiqueta de privacidad | Declaración de FGS en Play Console **por tipo, con vídeo** [149]; formulario Data safety |

**Conflicto abierto** [NV]: la página de Apple sobre el entitlement de Wi-Fi Aware solo dice "activa la capacidad en Xcode". Espressif (agosto de 2026) afirma que Apple debe aprobarlo para el equipo [43]. Hay que solicitarlo pronto.

### 4.10 Riesgos de revisión en tiendas

| Riesgo | Tienda | Mitigación |
|---|---|---|
| El revisor, con un solo dispositivo, no oye audio en background → rechazo 2.5.4 [135][136] | App Store | **Modo demo/loopback de un solo dispositivo** (un piloto simulado que habla tras bloquear la pantalla) + vídeo en las notas de revisión |
| Declarar `voip`/`push-to-talk` sin usar CallKit/PTT | App Store | Declarar solo los modos que se usan de verdad |
| Grabación sin indicación clara (2.5.14) | App Store | Paso explícito "Unirse", estado de micro persistente en la app, mute visible. El punto naranja del sistema siempre aparece [139] |
| La app exige push (4.5.4) si se usa PTT | App Store | El núcleo no puede depender de APNs |
| Vídeo de la declaración de FGS no válido | Play | `connectedDevice` exige mostrar "pairing and connecting to an external device directly inside the app" [149]. [NV] No se sabe si otro teléfono cuenta como "dispositivo externo". `microphone`/`phoneCall` pueden ser más fáciles de justificar |
| Fallo silencioso de audio en Android 17 tras un reinicio del FGS | Play QA | Reiniciar solo desde la UI o una notificación; usar Telecom; probar con `set-enable-hardening throw` |
| Los OEM matan el servicio | Campo | Onboarding con pasos específicos por OEM; watchdog con notificación para volver a unirse |

[H] Referente real: bitchat está en la App Store con live PTT (v1.7.0, julio de 2026) declarando solo `bluetooth-central` y `bluetooth-peripheral`. Su reproducción es **solo en primer plano** [137][179][185]. No se encontró ningún rechazo publicado específico de apps tipo intercom o walkie-talkie [NV].

### 4.11 Batería y temperatura (confianza baja, todo [I/NV])

| Componente | Corriente aproximada | %/h en 4.500 mAh |
|---|---|---|
| Micrófono + VPIO/AEC + Opus + reproducción | 40-80 mA | 1-2 % |
| BLE: 2-6 enlaces + escaneo/anuncio con ciclo útil | 5-20 mA | 0,1-0,5 % |
| Ruta de datos Wi-Fi Aware (realtime) | 30-100 mA | 0,7-2 % |
| Alojar LocalOnlyHotspot | 50-150 mA | 1-3 % |
| Pantalla encendida con navegación + GPS | 130-350 mA | 3-8 % |
| **Total con pantalla apagada** | — | **~2-5 %/h** |
| **Total con navegación en pantalla** | — | **~6-12 %/h** |

Base: valores de ejemplo de `power_profile` de AOSP, que **no son mediciones por dispositivo** [157]. Apple advierte que el modo `realtime` de Aware "can negatively impact battery drain" [37]. Existe riesgo térmico con el teléfono al sol, cargando, con la pantalla encendida y las radios activas. Hay que vigilar `thermalState` y el estado térmico de `PowerManager` para bajar bitrate y ciclo de escaneo.

---

## 5. Diseño de malla

### 5.1 Topología de convoy

- [I] Un convoy es una **cadena**. Con alcance por salto R y separación uniforme s, cada nodo ve unos **2·⌊R/s⌋ vecinos**:
  - R=60 m, s=20 m → grado 6 (agrupados).
  - R=60 m, s=100 m → grado 0: **la cadena se rompe**.
- Con 20-500 m de separación y R de decenas a ~100 m (sin medir), **las islas serán frecuentes en carretera abierta**.
- [I] Las velocidades relativas en el convoy son pequeñas; el Doppler a 2,4 GHz con 33 m/s es de ~270 Hz, despreciable. Lo que cambia los enlaces es la distancia, las curvas, las cuestas, la carrocería, el cuerpo del piloto y el bolsillo.
- [I] Un piloto que se detiene mientras el resto sigue a 120 km/h (33 m/s) sale de un radio de 100 m en ~3 s. Eso es más rápido que la detección de vecinos de OLSR (HELLO cada 2 s, hold 6 s) [196].
- **Requisitos** [I]:
  - Relés multi-salto.
  - Detección y visualización de islas ("quién me oye").
  - Avisos de partición.
  - Reunión automática.
  - Opcionalmente, fallback celular cuando haya cobertura.

### 5.2 Enrutamiento para voz

**Inundación gestionada sin estado** (patrón probado en Meshtastic [192][193] y bitchat [176][180]) [H/I]:

- **Cabecera de trama de voz** (borrador, ~10 B): `ver/tipo (1) | epoch/KID (1) | sender (2) | seq (4) | TTL (1) | edad en 10 ms (1)`, más una etiqueta AEAD de 16 B.
- **Deduplicación:** por `(sender, seq)` con ventana deslizante por emisor. bitchat usa un LRU de 1000 entradas y 5 min para mensajes; para voz basta una ventana corta.
- **TTL de voz:** ≤3 (máximo 4), por el presupuesto de latencia. Los mensajes de control pueden llevar un TTL mayor.
- **En cadenas** (grado ≤2): reenvío determinista; cada nodo interior debe reenviar. Gossip y MPR no ahorran nada aquí [H] [194][195].
- **En grupos agrupados** (grado ≥4):
  - Poda por tabla de vecinos: no reenviar al salto anterior ni a sus vecinos conocidos. Los enlaces BLE son unicast, así que la supresión "por escucha" de Meshtastic no aplica.
  - Supresión por contador con C=3-4 [195].
  - O un subconjunto determinista de ~log2(grado)+1 enlaces sembrado con el ID de mensaje, como hace bitchat [181].
- **Datos de referencia (corregidos)** [H]:
  - Gossip necesita p=0,65 en una malla regular de grado 6 y p≈0,86 en grado 3. En grafos aleatorios de grado medio 8, p=0,65-0,75.
  - Gossip ahorra hasta ~35 % de mensajes en redes densas, y se extingue con pocos vecinos [194].
  - Ni et al.: una retransmisión añade como máximo 61 % de cobertura (41 % de media), y ~0 tras oír 4 copias [195].
  - **Ambos estudios modelan radio broadcast compartida.** Sobre BLE unicast el coste es por enlace y hay que recalcularlo.
- **Acotar la latencia en los relés** [I]:
  - Jitter de relé de 0-10 ms (bitchat usa 8-25 ms para voz y 10-220 ms para mensajes [178][180]).
  - Colas por hablante de 2-3 tramas que descartan la más antigua.
  - Descartar tramas con edad acumulada > ~250 ms (umbral propuesto).
  - **No retransmitir nunca tramas en vivo.**
- **Sin enrutamiento proactivo** (OLSR/link-state): demasiado lento para este nivel de churn [196].
- **Límites de enlaces por nodo:** 3-6. Se re-evalúa por calidad del enlace (RSSI, pérdida, PHY).

### 5.3 Ancho de banda (cálculo [I])

**Tasa por flujo r** (capa de aplicación, con 26 B de cabecera + AEAD; las medias VBR reales son algo menores; la capa de enlace añade aproximadamente un 15-30 % estimado):

| Configuración | Payload/paquete | Paquetes/s | r | Con firma Ed25519 por paquete (+64 B) |
|---|---|---|---|---|
| Opus 16 kbps WB, 20 ms | 40 B | 50 | **26,4 kbps** | 52,0 kbps |
| Opus 16 kbps WB, 40 ms | 80 B | 25 | **21,2 kbps** | 34,0 kbps |
| Opus 24 kbps WB+LBRR, 20 ms | 60 B | 50 | **34,4 kbps** | 60,0 kbps |
| Opus 24 kbps, 40 ms | 120 B | 25 | **29,2 kbps** | 42,0 kbps |
| Opus 12 kbps, 40 ms | 60 B | 25 | **17,2 kbps** | 30,0 kbps |
| bitchat v1 (AAC-LC 16 kbps, 64 ms) | ≤210 B por ráfaga | ~15,6 | ~42 kbps en el aire (estimación del autor) [183] | n/a |

**Carga por enlace en cadena** = S × r, con S = hablantes simultáneos. Cada flujo cruza cada enlace una vez.

| S (hablantes) | r = 17,2 | r = 21,2 | r = 29,2 | ¿Cabe en BLE (≤ ~60 kbps = 50 % de ~120 kbps)? |
|---|---|---|---|---|
| 1 | 17 | 21 | 29 | Sí |
| 2 | 34 | 42 | 58 | Sí / justo |
| 4 | 69 | 85 | 117 | No (hace falta Wi-Fi) |
| 6 | 103 | 127 | 175 | No |
| 15 (todos con micro abierto) | 258 | 318 | 438 | No; en Wi-Fi multi-salto también es arriesgado |

- **Grupo agrupado** (parada, semáforo): el coste se traslada al emisor, que envía r × grado. Con grado 6 y r=21,2 son ~127 kbps. Sin poda, cada nodo reenvía a ~5 enlaces y se produce una tormenta.
- **Relés que mezclan** (decodificar, mezclar y recodificar) mantienen ~1 flujo por sentido y por enlace con cualquier S, a cambio de +1 trama de códec + look-ahead por salto y pérdida por codificación en tándem. Es una opción v2 [I].
- **Conclusión:** **S_max = 2 en BLE y ~4 en Wi-Fi.** No hay open-mic para todos.

### 5.4 Control de turno (floor control)

[I], diseño propuesto, coherente con la precedencia de Cardo ("first N speak") [165]:
- v1 usa un **turno blando**.
  - Cualquier miembro puede hablar.
  - Los relés imponen el tope S_max por hablantes *activos* (por eventos de inicio de VAD/PTT).
  - Prioridad: capitán de ruta o líder primero, después el que empezó antes (`(prioridad, timestamp de inicio, node_id)`).
- Quien exceda el tope oye un **tono de "ocupado"**, y la UI muestra "canal lleno".
- Las ráfagas de voz llevan `burst_id` + marcas START/END, como el `VoiceBurstPacket` de bitchat [182].
- v2 podría añadir un turno estricto con token gestionado por el líder.

### 5.5 Seguridad

**Modelo de amenazas:**
- Escuchas de terceros cercanos.
- Suplantación por externos e internos.
- Rastreo de pilotos por anuncios BLE.
- Denegación de servicio con paquetes manipulados.

Lecciones de Bridgefy, roto dos veces en 2021 y 2022 [H] [190][191]:
- Rastreo de usuarios.
- Suplantación y MITM.
- Descifrado de broadcasts sin la clave compartida.
- Caída de toda la red con una bomba de descompresión.

**Diseño propuesto** [I, sobre especificaciones verificadas]:

1. **Unirse por QR o deep link.** El líder crea el grupo con `group_id` + **secreto de grupo de 256 bits** + su clave pública. Es el equivalente a "Remote Grouping" de Cardo (que comparte un enlace) [163] y al QR de Sena para emparejar Bluetooth [170].
2. **Sesión por enlace:** **Noise XXpsk3**, con el secreto de grupo como PSK, que debe tener 256 bits de entropía. Alternativa: **KK** para re-handshakes entre miembros conocidos [198]. Implementación: `snow`, que sigue la revisión 34 de la especificación y **no tiene auditoría formal** [241].
3. **Claves de medio por emisor estilo SFrame** (RFC 9605) [197]:
   - `sender_key = HKDF(epoch_key, sender_id)`, con ChaCha20-Poly1305 o AES-GCM.
   - `nonce = salt ⊕ seq`.
   - Cabecera como AAD.
   - Ventana anti-replay.
   - "A given base_key MUST NOT be used for encryption by multiple senders" [H].
4. **Evitar reutilizar nonces:** si el contador se reinicia (reinstalación o crash), se rompe AEAD. Hay que persistir el contador **o** generar una sub-clave de sesión aleatoria por arranque, incluida en el KID.
5. **Rekey:** **el líder es el único que hace commit**. Al expulsar a alguien, el líder distribuye una nueva `epoch_key` por las sesiones Noise pareadas. Es O(N), trivial con N≤31. MLS (RFC 9420) exige resolver commits en conflicto y encaja mal en una malla particionable [199]. MLS (por ejemplo OpenMLS) queda como mejora posterior.
6. **Suplantación interna:** SFrame no autentica por emisor; cualquier miembro puede forjar tramas de otro [197]. Si importa, se **firma por ráfaga** con Ed25519, no por trama, porque la firma por trama dobla el ancho de banda (tabla §5.3).
7. **Privacidad:** una dirección BLE aleatoria no protege si el payload lleva tokens estables [H] [200].
   - bitchat admite que su peer ID de 8 B es estable y enlazable [176].
   - En v1, un UUID de servicio fijo revela "aquí hay un usuario de mesh-app". La identidad solo se revela tras conectar, dentro de Noise.
   - En v2 se puede evaluar un UUID de servicio rotatorio derivado del grupo, del tipo `HMAC(group_key, ⌊t/10 min⌋)`. Los iPhone en background deben escanear UUIDs explícitos, así que habría que buscar el UUID de la época actual y el de la siguiente y re-arrancar el escaneo en cada rotación.
   - Las listas de vecinos deben ir autenticadas y cifradas desde el principio; añadirlo después es difícil.
8. **Evitar diálogos de emparejamiento del SO:** usar GATT sin cifrar y L2CAP CoC inseguro, con toda la autenticación en la capa de app [I]. En convLink, un CCCD cifrado lanzaba el diálogo de PIN de iOS [L-pitfalls].
9. **Robustez:**
   - Parsers acotados y sin compresión.
   - Fuzzing (cargo-fuzz o libFuzzer) de todos los parsers.
   - Límites de reensamblado como los de bitchat: 8 ensamblados concurrentes y 384 KiB por ráfaga [182].
10. **Wi-Fi Aware en iOS 26.4+:** `WAConnection.deriveSharedSecret` (derivación de Wi-Fi Aware 5.0) puede vincular TLS/QUIC sin intervención del usuario. QUIC sobre Aware requiere iOS 27 [38][41].

**Precedente reutilizable:** convLink ya rota claves de grupo por época, con un *lease* offline de 24 h y rotación en cambios de roster (RPC `get_convoy_mesh_context`) [L5]. Es la base natural si mesh-app se integra con convLink (§8).

---

## 6. Referentes de producto y arte previo

### 6.1 Cardo y Sena

- **Cardo DMC, 2ª generación (Packtalk Pro/Edge/Neo)** [H] [160][161][162][163]:
  - 1,6 km entre unidades y 8 km de red.
  - **31 pilotos y pasajeros** tras Mesh Boost (OTA gratuita, agosto de 2026). Grupos mayores se dividen.
  - Se re-enruta solo; el piloto que cae se reincorpora "sin botones, sin app, sin emparejar".
  - Public Mesh (hablar espontáneamente con usuarios DMC cercanos).
  - Remote Grouping (enlace del líder en Cardo Connect).
  - Elegibles: PACKTALK PRO, EDGE y NEO. Las versiones Harley-Davidson y Schuberth quedan fuera hasta el Q4 de 2026.
  - DMC solo funciona entre Cardo. Entre marcas se usa una cadena Bluetooth de 2-4 pilotos sin auto-reparación.
  - "Hey Cardo, mute microphone" [166].
- **Hablantes simultáneos en Cardo** [H, matiz]: en la 1ª generación DMC aparecen tanto 4 ("the first 4 that speak") como 6 (firmware v2.0, 2016) [164][165]. **El tope para la 2ª generación y Mesh Boost es desconocido.** No usan PTT. [H] El plan `VOZ_MESH_PLAN.md` (línea 21) afirma que "Cardo itself falls back to PTT for groups", lo cual **es falso** [L2].
- **Sena 60S** (noviembre de 2024) [H] [167][168][169][170]:
  - Mesh 3.0, conmutable a 2.0 (incompatibles entre sí).
  - Group Mesh hasta 24 y Open Mesh "virtually unlimited" por canal.
  - 2 km en terreno abierto, Bluetooth 5.3, 17 h de conversación en Mesh.
  - WAVE: intercom celular por VoIP con paso automático Wave↔Mesh.
  - Hay discrepancia en los canales de Open Mesh: 9 según la comunidad y 6 según una extracción de la página del 60S [NV].
  - [NV] MeshON (junio de 2026) une cualquier casco BT a Sena Mesh a ~800 m.
- **Quejas de los usuarios** (anecdóticas) [NV] [170][171][172][173]:
  - Caídas tras colinas, edificios y cañones.
  - Estática, eco y voces en bucle en grupos de más de ~4 (Sena).
  - Incompatibilidad entre generaciones de mesh.
  - Controles engorrosos.
- **Lo más valorado:** agrupar sin esfuerzo y la reincorporación automática.

### 6.2 Tabla de paridad de funciones

| Función | Referente Cardo/Sena | mesh-app | Notas |
|---|---|---|---|
| Malla con relés que se auto-repara | Sí | **v1** | Inundación gestionada; sin rutas que mantener |
| Reincorporación automática | Sí, sin botones | **v1** (con huecos de segundos) | La reconexión BLE tarda "varios segundos" [174] |
| Grupo privado por invitación | Enlace (Cardo), QR (Sena BT) | **v1**: QR / deep link | 256 bits de secreto |
| Canales públicos | Public Mesh / Open Mesh | Después (v1.x) | Clave derivada del canal; anti-spam |
| Tamaño de grupo | 31 / 24 | **v1: 2-15**; 31 después | Depende de hablantes, no de miembros |
| Hablantes simultáneos | 4-6 (1ª gen.) | **v1: 1-2 en BLE**; ~4 en Wi-Fi | Lo imponen los relés |
| Manos libres abierto | Sí (full-duplex + mute) | **v1 como VOX con tope** + PTT | Open-mic para todos: inviable en BLE |
| Alcance por salto de 1,6-2 km | Sí | **Inviable en teléfonos** | Radios de teléfono |
| Alcance de grupo de 8 km | Sí | Parcial | Solo si la separación es menor que el alcance |
| Unir pilotos de otras marcas | Cardo Bridge, Sena MeshON | **Inherente** (cualquier casco emparejado al teléfono) | No puede unirse a DMC/Sena Mesh (radios propietarias) |
| Compartir música | Sí (pasajero) | Inviable o después | iOS no puede capturar el audio de otras apps |
| Multitarea de audio (música + GPS + intercom) | Sí | Parcial | HFP abierto degrada la música a mono (§3.5) |
| Comandos de voz | "Hey Cardo", Sena | Después | Siri/App Intents/Assistant |
| Detección de choques | Cardo (requiere datos) | Después | Los teléfonos tienen sensores |
| Fallback celular | Sena WAVE | Después (diferenciador) | Reutilizar el backend Supabase de convLink |
| **Mapa de posiciones del grupo** | No | **v1, diferenciador** | GPS del teléfono |
| Autonomía de 13-17 h | Sí | No equivalente | Cargador en el vehículo |
| Interoperabilidad iOS/Android y entre cascos | No | **Ventaja** | — |

### 6.3 Arte previo y lecciones

| Proyecto | Radio | iOS↔Android | Voz | Lección |
|---|---|---|---|---|
| **bitchat** (2025-26) | Malla BLE GATT, TTL 7; Nostr con internet | Sí ("fully protocol-compatible") [8] | **PTT en vivo desde julio de 2026**: AAC-LC 16 kHz/16 kbps, tramas de 64 ms, ≤210 B, sin ACK ni retransmisión, jitter de relé 8-25 ms, buffer de 350 ms, ~0,5 s a 1 salto (dato del autor); al soltar, pasa a nota de voz .m4a [177][178][182][183][184] | Demuestra que la voz BLE multiplataforma funciona. La reproducción es **solo en primer plano** y con la conversación en pantalla [179]. Peer ID estable. Sin auditoría externa mencionada. Wi-Fi Aware solo en Android y desactivado por defecto [9] |
| Bridgefy SDK | Malla BLE | Sí | No | Roto dos veces: rastreo, suplantación, MITM, DoS [190][191] |
| FireChat (2014) | Multipeer (iOS) y otra malla en Android | No | No | Las plataformas no interoperaban; ~30 ft confirmados [175] |
| Briar | BT/Wi-Fi/Tor | Solo Android | No | Los límites de background de iOS impidieron el port [186] |
| Berty | BLE + Multipeer + Nearby | Solo vía BLE | No | BLE "extremely slow"; BLE inestable en Android [187] |
| Ditto (comercial) | BLE + AWDL + Wi-Fi Aware + LAN | BLE confirmado | No | Pocas conexiones BLE; conexión de "varios segundos" [174] |
| Google Nearby Connections | BLE/L2CAP, AWDL, Aware, LAN, hotspot (código Apple) | En evolución | No | El README dice que iOS solo soporta Wi-Fi LAN [3]; la guía Swift menciona hotspot [4]. No es una base fiable hoy |
| Serval Mesh | 802.11 ad-hoc | Solo Android (root) | **Sí (VoMP)** | Voz multi-salto posible, pero requería root [189] |
| goTenna Pro | Radio externa | Vía dispositivo | No | Solo texto y ubicación [188] |
| Meshtastic | LoRa, inundación gestionada | Vía dispositivo | No | Patrón de deduplicación, hop limit y retardo por SNR [192] |
| Zello / Sena WAVE | Internet | Sí | Sí | No funcionan offline |

**Lecciones** [I]:
1. BLE es el único puente multiplataforma demostrado.
2. Los límites de background de iOS mataron proyectos (Briar) o redujeron funciones (bitchat solo reproduce en primer plano). Es el riesgo n.º 1 de la experiencia de usuario.
3. La seguridad "hecha en casa" falla: hay que autenticar cada broadcast y no usar una clave global para toda la app.
4. Nadie ha publicado voz multi-salto entre teléfonos con pantalla bloqueada y casco. mesh-app sería territorio nuevo y requiere medir.

---

## 7. Framework y arquitectura

### 7.1 El hecho decisivo

[H] Todo lo crítico solo es accesible en el lenguaje nativo de cada SO:
- Wi-Fi Aware de iOS es **solo Swift** (structs + `NetworkConnection`). Kotlin/Native no puede importar módulos Swift puros [34][218].
- CoreBluetooth y AVAudioSession son APIs de Apple.
- BLE y Wi-Fi Aware de Android son APIs de Java; el NDK no tiene módulo BT.
- El FGS de Android y los modos de background de iOS son nativos.
- El JS de WKWebView se detiene en background [NV para iOS 26].

**Consecuencia:** con cualquier framework, el motor es un **SDK nativo independiente de la UI** que gestiona su propio ciclo de vida. La UI es un cliente desechable.

### 7.2 Comparación puntuada de frameworks de UI

Escala 1-5. Total ponderado sobre 100. Puntuación del autor del informe [I].

| Criterio (peso) | Capacitor 8→9 | React Native 0.88 / Expo SDK 58 | Flutter 3.47 | KMP + Compose MP 1.11 | .NET MAUI 10 | Nativo SwiftUI + Compose |
|---|---|---|---|---|---|---|
| Ergonomía de módulos nativos (15) | 4 | 5 (Expo Modules, Nitro, uniffi-bindgen-react-native) | 4 | 3 (Swift export en Alpha; interop solo con ObjC) | 2 | 5 |
| Motor en FGS/background con UI suspendida (15) | 4 (patrón probado en convLink `:drive`, solo Android) | 4 | 3 | 4 | 2 (pausas del GC) | 5 |
| Eventos de alta frecuencia a la UI (10) | 3 (JSON, agrupar a ≤15 Hz) | 5 (JSI) | 4 | 5 | 4 | 5 |
| Build/CI (10) | 4 | 3 (rotación de SDK) | 3 | 3 | 2 | 3 |
| Testing (10) | 4 (Vitest/Playwright contra el simulador WASM) | 3 | 4 | 4 | 3 | 3 |
| Reutilizar habilidades React/TS (15) | 5 | 4 | 1 | 1 | 1 | 1 |
| Compartir componentes/tokens con convLink (10) | 5 | 3 (NativeWind v5 sigue en RC [213]) | 1 | 1 | 1 | 1 |
| Calidad de UI en gama baja (10) | 3 (WebView) | 4 | 4 | 4 | 3 | 5 |
| Ecosistema/longevidad (5) | 4 | 5 | 4 | 4 | 2 (soporte hasta mayo de 2027 [219]) | 5 |
| **Total** | **81** | **80** | **60** | **62** | **43** | **72 (doble trabajo de UI)** |

**Lectura** [I]:
- Capacitor y RN están **empatados en la práctica**. Capacitor gana solo por reutilización con convLink.
- Como el adaptador de UI es fino (~300 LOC por SO), cambiar a RN es barato. Se decide con una **prueba de rendimiento en la semana 1** (§9, T8).
- Flutter, KMP y MAUI renuncian a las habilidades React sin resolver ningún problema nativo difícil. KMP y MAUI añaden riesgo de pausas del GC en audio [217].

**Hechos de calendario** [H]:
- Capacitor 8.5.3 es la última estable; convLink usa 8.4.1.
- **Capacitor 9 se espera para finales de noviembre de 2026** (9.0.0-alpha.8 en npm). Eliminará la distribución XCFramework y obligará a los autores de plugins a ejecutar herramientas de migración [204].
- El trunk de CocoaPods pasa a solo lectura el **02-12-2026** [206].
- El soporte extendido de Capacitor 7 termina el 08-12-2026 [205].
- **Implicación:** usar SPM desde el día 1 y aislar el motor como Swift Package + AAR detrás de un plugin fino, o arrancar ya sobre Capacitor 9.

### 7.3 Núcleo compartido

| Criterio | Rust + UniFFI (C ABI en el camino de tiempo real) | C++ (JNI/Djinni + interop C++ de Swift) | Kotlin compartido (KMP) | Swift + Kotlin duplicados |
|---|---|---|---|---|
| Opus | 5: libopus 1.6.1 vía `opusic-sys` 0.7.5 (features `dred`/`osce`); el crate `opus` 0.4.0 ya depende de él [235][236] | 5 | 2 | 2: wrappers Swift abandonados [237][238] |
| AEC/NS/AGC | 3: DSP de plataforma + `sonora` (joven) | 5: WebRTC APM nativo | 2 | 3 |
| Cripto | 5: RustCrypto/ring + `snow` (sin auditoría) | 4 | 3 | 2: CryptoKit frente a JCA, interop byte a byte |
| Simulador de malla en CI | 5: sans-IO, `cargo test`, también WASM | 4 | 4 (JVM) | 1 |
| Bindings | 5: Kotlin/Swift nativos; RN, Flutter, WASM | 3 | 3 | n/a |
| Tiempo real sin GC | 5 | 5 | 2 | 4 |
| Seguridad de memoria con entrada de radio no confiable | 5 | 2 | 4 | 4 |
| Tamaño binario | 4 (~1 MB comprimido es alcanzable [245]) | 5 | 3 | 5 |
| Familiaridad del desarrollador | 2 | 4 (firmware ESP32) | 3 (Kotlin de convLink) | 3 |
| **Total /45 (sin build)** | **39** | **37** | **26** | **26** |

(El informe original puntuaba sobre 55, incluyendo "build tooling": Rust 47, C++ 44, KMP 32, duplicado 36. La conclusión es la misma.)

**Correcciones relevantes** [H]:
- UniFFI 0.32.2 (23-09-2026) ya pasa `&[u8]` **sin copia** desde Swift `Data` y desde `ByteBuffer` directo de Kotlin, solo en funciones síncronas. Es un **cambio incompatible** en Kotlin [230].
- `&mut [u8]` sin copia y un bindgen de Kotlin basado en JNI están en `main`, sin publicar.
- UniFFI sigue siendo pre-1.0: hay que fijar la versión [229].
- La cifra de ~1,4 µs por llamada procede de BoltFFI, un competidor, sin metodología publicada [233].
- La regla no cambia: **los callbacks de CoreAudio y Oboe solo llaman funciones `extern "C"` que tocan anillos SPSC preasignados** (`rtrb` 0.4.0) [234].
- `uniffi-bindgen-react-native` (npm 0.31.0-6) va por detrás de UniFFI 0.32 [211].

**Alternativas al núcleo Rust:**
- **C++**, si el desarrollador no adopta Rust: misma arquitectura, con fuzzing obligatorio [247].
- **Swift en Android**: Swift 6.3 (marzo de 2026) publicó el primer SDK oficial de Swift para Android, con interop vía `swift-java`/JNI. Es creíble, pero el toolchain Android es joven [248][249].

**Testing** [H/I]:
- El núcleo sans-IO (patrón de `str0m` [242]) permite un **simulador determinista de 15 nodos en movimiento** en CI, con semilla fija.
- `turmoil` no trae parámetros de latencia y pérdida integrados, así que el modelo de radio se escribe aparte [243].
- El determinismo exige eliminar fugas de reloj, `getrandom` y el orden de `HashMap` [244].
- **El simulador no modela** la planificación BLE del SO, el throttling de iOS, el NDP de Aware, el ruteo HFP ni los gestores de batería de los OEM. Las pruebas de campo siguen siendo obligatorias.

### 7.4 Disponibilidad de librerías (octubre de 2026)

| Necesidad | Estado | Decisión |
|---|---|---|
| BLE central | react-native-ble-plx, flutter_blue_plus (licencia de pago para uso comercial), Kable y `@capacitor-community/bluetooth-le` son **solo central** [220][221][222][223] | Nativo |
| BLE periférico / servidor GATT / L2CAP | munim-bluetooth **0.10.2** (Nitro; ambos roles; L2CAP; un solo mantenedor; advierte que en iOS 26+ el relanzamiento depende de AccessorySetupKit) [224]; bluetooth_low_energy 6.2.1 (sin L2CAP) [225]; kmp-ble (7 estrellas, minSdk 33) [226]; Capawesome (de pago) [227] | **Nativo** (CBPeripheralManager / BluetoothGattServer / CBL2CAPChannel / L2CAP CoC) |
| Wi-Fi Aware | Solo `react-native-wifi-aware` 0.1.0 (0 estrellas; validado en Android; iOS solo compila; interop "not implemented") [66] | **Nativo** |
| Opus | Wrappers Swift/Android obsoletos | libopus 1.6.1 en el núcleo |

### 7.5 Qué vive dónde

| Capa | Responsabilidades | Por qué ahí |
|---|---|---|
| **Núcleo compartido** (Rust, sans-IO, sin hilos ni E/S) | Protocolo versionado; grupo/sesión; identidad Ed25519; Noise; AEAD por trama y anti-replay; malla (vecinos, calidad de enlace, inundación + dedup + TTL + jitter, poda, control de tasa); voz (Opus, FEC/DRED/PLC, jitter buffer por hablante, mezclador, VAD/VOX, turno); temporizadores vía `handle_timeout(now)`; métricas | Una sola implementación garantiza compatibilidad iOS↔Android; simulación determinista; WASM para la UI |
| **Runtime del núcleo** | Hilo del motor (tick de 5 ms), colas, anillos `rtrb`, `extern "C"` `mesh_audio_capture/render`, exports UniFFI (Engine, traits foráneos RadioSink/EventSink) | Conecta el núcleo con los hilos del SO |
| **Capa de plataforma** (Swift Package + AAR Kotlin) | Audio (AVAudioSession/VPIO; Oboe + VOICE_COMMUNICATION / Core-Telecom); ruteo e interrupciones; BLE central + periférico + L2CAP; Wi-Fi Aware; host del servicio (FGS / modos de background); botones del casco; Keychain/Keystore; permisos; señales térmicas | APIs solo nativas y críticas para el ciclo de vida |
| **Adaptador de UI** | Plugin Capacitor `MeshIntercom` (~300 LOC por SO): permisos, start/stop/bind, paso de `command(json)` y eventos `snapshot` (≤15 Hz). Plan B: Expo Module | Fino para poder cambiar de framework |
| **UI** (React 19 + TS + Zustand + Tailwind v4) | Unirse por QR, miembros con indicador de quién habla, PTT/VOX, selector de ruta de audio, diagnóstico, mapa | Habilidades y reutilización; **nunca en el camino de tiempo real** |

```
UI (reemplazable): Capacitor WebView — React 19 + TS + Zustand + Tailwind v4
   tipos TS generados desde Rust (ts-rs) · impl web del plugin = núcleo.wasm + simulador
        ▲ snapshots JSON ≤15 Hz                 │ comandos JSON
ADAPTADOR: plugin Capacitor "MeshIntercom" (~300 LOC/SO)   [Plan B: Expo Module/Nitro]
        ▲ UniFFI (plano de control, async, traits foráneos)
CAPA DE PLATAFORMA = MeshEngine SDK (Swift Package | AAR Android)
   ServiceHost (FGS microphone|connectedDevice · iOS audio+bluetooth-*) · AudioDriver (VPIO | Oboe/Telecom)
   RadioDrivers (BLE central+periférico+L2CAP · Wi-Fi Aware) · KeyStore
        ▲ hilos RT: extern "C" capture/render     ▲ paquetes y eventos de enlace (UniFFI, ~50-400/s)
          (anillos SPSC rtrb, sin alloc ni locks)
NÚCLEO COMPARTIDO (Rust, sans-IO): runtime · cripto · malla · voz · protocolo
   → cargo test: simulador determinista de 15 nodos · → wasm32 · → aarch64-android / aarch64-ios
```

### 7.6 Tres opciones de arquitectura concretas

**Opción A (recomendada): Capacitor + MeshEngine SDK nativo con núcleo Rust; BLE como base y subida a Wi-Fi Aware**
- UI en Capacitor 8→9, compartiendo paquetes de UI y tokens con convLink vía pnpm workspace.
- Motor nativo (Swift/Kotlin) con núcleo Rust sans-IO.
- Transporte:
  - BLE L2CAP CoC inseguro + Noise como base común.
  - Subida a Wi-Fi Aware iPhone↔iPhone (pares pre-emparejados en el lobby) y Android↔Android.
  - iPhone↔Android Aware solo con una lista blanca empírica de modelos.
  - Hotspot Android opcional (sin coexistir nunca con Aware en el mismo Android).
- Voz PTT/VOX con S_max 2 en BLE y ~4 en Wi-Fi.
- **Pros:**
  - Máxima reutilización de convLink.
  - Un solo protocolo: imposible que iOS y Android diverjan.
  - Simulador en CI y en el navegador.
  - El camino de audio queda fuera de cualquier runtime con GC o JS.
  - convLink podría adoptar el plugin más adelante.
- **Contras:**
  - Curva de aprendizaje de Rust.
  - UniFFI pre-1.0.
  - Migración a Capacitor 9 inminente.
  - El rendimiento de WebView en gama baja no está medido.
  - Dos capas nativas de ~3-6k LOC cada una [I].
  - Wi-Fi Aware añade UX de PIN por par y no resuelve iPhone↔Android.

**Opción B: Expo / React Native + el mismo MeshEngine SDK**
- Idéntica a A, pero con UI en Expo SDK 58 / RN 0.88 y un Expo Module o Nitro como adaptador.
- **Pros:**
  - Mejores eventos de alta frecuencia (JSI).
  - UI nativa y más fluida en gama baja.
  - Herramientas de módulos nativos más fuertes.
  - El núcleo, la capa de plataforma y la mayor parte de la lógica TS se reutilizan tal cual.
- **Contras:**
  - Se pierde el uso compartido de componentes DOM y Radix con convLink.
  - NativeWind v5 sigue en RC.
  - Rotación de SDK de ~3 versiones al año.
  - Hermes y Metro añaden mantenimiento.

**Opción C: motor duplicado en Swift + Kotlin, sin núcleo compartido (estilo bitchat)**
- UI Capacitor y motor escrito dos veces (Swift y Kotlin), cada uno con su Opus, su jitter buffer y su cripto (CryptoKit frente a JCA/Tink).
- **Pros:**
  - Sin Rust ni FFI.
  - Arranque más rápido para un prototipo de un solo salto.
  - Precedente real: bitchat-android es un port del de iOS [8].
- **Contras:**
  - Doble trabajo en el componente más difícil (jitter buffer, FEC, mezclador, enrutamiento).
  - **Divergencia del protocolo en el cable**, que se convierte en el modo de fallo dominante.
  - Sin simulador compartido.
  - Wrappers de Opus obsoletos.
  - Interop criptográfica byte a byte.
- Útil solo como prototipo desechable de la fase 0.

(Variantes de A: núcleo en C++ si se rechaza Rust; núcleo en Swift compilado para Android con el SDK oficial. Ambas son viables con la misma separación en capas.)

---

## 8. Qué reutilizar de comvoy-app (convLink)

**Contexto verificado del repo** [L]:
- convLink es una app de navegación para convoyes (coches y motos) sin chat de texto (eliminado a propósito). Solo quedan alertas `ALT:`.
- Stack: React 19.2.7, Vite 8.1.4, TS 5.9 strict, Tailwind 4.3 + shadcn/ui (new-york, zinc), Zustand 5, i18next EN/ES, Vitest 4, Capacitor 8.4.1, pnpm workspace.
- **Backend: Supabase desde el 13-09-2026.** La memoria del usuario aún dice PocketBase; el código manda.
- BLE con `@capacitor-community/bluetooth-le` 8.2.0, **solo central**, hacia un ESP32 por NUS.
- La voz **nunca se construyó**. `research/VOZ_MESH_PLAN.md` es un plan para ESP32/ESP-NOW.

### 8.1 Convenciones a heredar

- **pnpm 10+ / Node 22.18+ exclusivamente.** Workspace con `packages/*` y plugins locales `workspace:*`. Superficie de scripts pequeña (`dev`, `build`, `lint`, `test`) más un único runner `scripts/tasks.mjs` con `--dry-run` [L13].
- TS strict con `noUnused*`, `verbatimModuleSyntax`, `erasableSyntaxOnly`, alias `@/*`.
- ESLint 9 flat (React Compiler en *warn*, `catch {}` siempre con comentario).
- React Compiler activado. Sin StrictMode.
- Carpetas por dominio. Módulos de política puros y testeables separados de hooks y stores.
- Zustand: un store por dominio; `persist` con `name`/`version`/`migrate`/`merge`. El código no-React usa `getState()`.
- **i18n:** exactamente `en` y `es`, con `defineCatalog(en, es)` que hace fallar la compilación si falta paridad de claves [L10].
- Tests colocados junto al código (`*.test.ts`) y **tests de contrato estático** que parsean Info.plist/pbxproj/manifest (p. ej. `tests/ios-scene-lifecycle.test.mjs`) [L9].
- **Documentación viva:**
  - README = estado actual.
  - BITACORA = registro de decisiones, más reciente primero, con "Invariantes — NO REVERTIR".
  - `research/` = propuestas marcadas "no implementado".
  - Separar siempre las verificaciones automáticas de las físicas ("pendiente físico, no fingido").
- **Reglas de producto:** solo vehículos, nada de escribir mientras se conduce, objetivos táctiles de 44 px, distracción mínima. La voz es el canal sancionado.

### 8.2 Design system

[L11] **Estética "Apple-inspired"** sobre Tailwind v4 + shadcn new-york/zinc. Tokens en `src/App.css`, líneas 1-425:
- **Colores:**
  - Claro: `#f2f2f7` / `#1c1c1e` / `#ffffff`.
  - Oscuro: `#08080a` / `#f5f5f7` / `#1c1c1e`.
  - 8 presets de acento en oklch (verde por defecto) en `src/map/style/theme-presets.ts`; conviene moverlo fuera de `map/`.
- **Tipografía:** stack del sistema (SF Pro); `.ios-page-title` 1,75 rem/650.
- **Forma y espaciado:** `--radius` 1 rem; cápsulas; **mínimo táctil 2,75 rem**.
- **Materiales:** `.glass` / `.glass-strong`, sin blur en Android (`html.android`) y opacos con `prefers-reduced-transparency`.
- **Movimiento:** `--ease-ui` y `--duration-ui` de 220 ms, con `prefers-reduced-motion`.
- **Componentes shadcn:** button, dialog, confirm-dialog, sheet, drawer (vaul), slider, popover, sonner.
- **Marca:** `#0b1117` / `#42e37b`. Iconos generados desde un SVG con `scripts/generate-brand-assets.swift`.
- **Recomendación** [I]: extraer tokens y primitivas a un paquete compartido (`@convlink/tokens`, `@convlink/ui`), consumido por ambas apps.

### 8.3 Código reutilizable (rutas)

| Qué | Rutas (en `/Users/ericksosagarcia/Desktop/personal/loRa/comvoy-app/`) | Cómo |
|---|---|---|
| Petición/respuesta BLE con un solo vuelo en curso (generación + requestId; instalar el listener antes de escribir) | `src/ble/bleRequests.ts` | Copiar para el plano de control |
| Framer con fragmentación y límites estrictos + helpers WebCrypto (P-256, HMAC, SHA-256, comparación en tiempo constante) | `src/ble/convlinkProtocol.ts`, `src/ble/convlinkProtocol.test.ts` | Reutilizar la técnica y los tests (no el formato 0xC2 ni las tramas de 20 B) |
| Política de descubrimiento BLE entre plataformas | `src/ble/discovery.ts`, `src/ble/discovery.test.ts`, `src/ble/useBle.ts` (`getNativeScanOptions` ~l.64) | Portar las reglas al escáner nativo |
| Patrones de ciclo de vida de conexión (generaciones, barrera de limpieza, backoff 300 ms→10 s, vitalidad = cualquier trama entrante, sondeo al volver a primer plano) | `src/ble/useBle.ts` (~l.406-760, 1006-1050), `src/ble/bleIdentityBoundary.ts` | Copiar **los patrones**, no la estructura de 1.627 líneas |
| Normalización de errores de emparejamiento | `src/ble/pairingRecovery.ts` | Si se usan atributos cifrados |
| Estado foreground/background, reconexión coalescida, keep-awake, políticas de cadencia, estado de red | `src/hooks/useAppState.ts`, `src/lib/syncReconnect.ts`, `src/hooks/useKeepScreenAwake.ts`, `src/lib/backgroundRuntimePolicy.ts`, `src/lib/networkStatus.ts` | Casi literal |
| Bóveda segura nativa (Keystore AES-256-GCM con AAD; Keychain solo-dispositivo) | `android/app/src/main/java/com/convlink/app/ConvLinkVaultPlugin.java`, `ios/App/App/ConvLinkVaultPlugin.swift`, `src/lib/convlinkVault.ts`, `android/app/src/main/res/xml/convlink_backup_rules.xml`, `.../convlink_data_extraction_rules.xml` | Renombrar; guardar el par de claves de identidad y las claves de grupo |
| Registro de plugins locales (Android `registerPlugin` antes de `super`; iOS `registerPluginInstance` en `capacitorDidLoad`) | `android/app/src/main/java/com/convlink/app/MainActivity.java`, `ios/App/App/BackgroundLocationPlugin.swift`, `ios/App/App/Base.lproj/Main.storyboard` | Plantilla |
| Ciclo de vida UIScene de iOS 27 + test de contrato | `ios/App/App/SceneDelegate.swift`, `ios/App/App/AppDelegate.swift`, `ios/App/App/Info.plist`, `tests/ios-scene-lifecycle.test.mjs` | Copiar justo después de `cap add ios`; ampliar el test a las claves de micro y audio |
| Plantilla de plugin Capacitor de workspace | `packages/convlink-auto/{package.json,tsconfig.json,src/index.ts,src/definitions.ts,src/web.ts,android/build.gradle,android/src/main/AndroidManifest.xml}` | **Añadir** `ios/` + `Package.swift` raíz (SPM); modernizar Gradle (JVM 21, Kotlin 2.3) |
| Foco de audio Android para avisos de voz | `packages/convlink-auto/android/src/main/java/com/convlink/auto/ConvLinkAutoPlugin.kt`, `.../drive/NativeGuidanceAnnouncer.kt` | Referencia; la voz usará `USAGE_VOICE_COMMUNICATION` |
| Capa TTS (prioridad, deduplicación, desbloqueo web) | `src/gps/useTTS.ts`, `src/i18n/catalogs/voice.ts` | Avisos hablados ("Ana se unió", "canal ocupado") |
| Infraestructura i18n tipada | `src/i18n/{instance.ts,config.ts,index.ts,languages.ts,resources.ts,formatters.ts,LocaleEffects.tsx}`, `src/i18n/catalogs/defineCatalog.ts`, `src/i18n/i18n.test.ts` | Copiar; quitar los campos de Google/Valhalla |
| Design system | `src/App.css`, `src/map/style/theme-presets.ts`, `src/store/useSettingsStore.ts` (tema), `src/components/ui/`, `src/lib/utils.ts`, `components.json`, `src/main.tsx`, `tests/frosted-material.test.mjs` | Ver §8.2 |
| Hápticos y helpers de plataforma | `src/lib/haptics.ts`, `src/lib/platformAccess.ts`, `src/lib/useBackButton.ts` | Hápticos para pulsar y soltar PTT |
| Grabador de campo y diagnóstico (anillos O(1) acotados, eventos saneados) | `src/gps/gpsRecorder.ts`, `src/ble/convlinkDiagnostics.ts`, `docs/CONVLINK_FIELD_TESTING.md` | Modelo para el grabador de voz/malla de la fase 0 |
| Identidad offline de confianza + outbox duradero | `src/lib/trustedOfflineIdentity.ts`, `src/lib/durableOutbox.ts`, `src/lib/backendSession.ts` | Identidad usable sin red |
| Tooling | `scripts/tasks.mjs`, `scripts/tasks.test.mjs`, `scripts/check-backend-boundary.mjs` (como idea: prohibir audio por el bridge JS), `scripts/generate-brand-assets.swift`, `scripts/android-build-release.sh` | Adaptar |
| Parámetros de voz ya investigados | `research/VOZ_MESH_PLAN.md` (§0, 1.1, 1.3, 4, 5, 7) | Opus 16k/20 ms, cabecera de inundación, dedup ~512/~2 s, TTL 3-4, jitter de 2-8 ms. **Corregir** la afirmación sobre PTT de Cardo y el 24 kbps fijo |

### 8.4 Puntos de integración con convLink

1. **Identidad:** el mismo proyecto Supabase (Auth + `public.profiles`) para nombres y tipo de vehículo. La voz debe funcionar sin red, con identidad offline de confianza y un par de claves en la bóveda [L6].
2. **Convoy:** `public.trips` (`invite_code` de 8 caracteres) y `trip_members`, con RPCs `create/join/leave_convoy` [L7]. **El servidor limita el convoy a 5 miembros** (y el roster de malla a 10), frente a los 2-15 de mesh-app. Hace falta cambiar ese límite o crear un concepto separado de "grupo de voz". Requiere una migración en comvoy-app (decisión del propietario).
3. **Claves de grupo:** precedente en `comvoy_private.convoy_mesh_epochs` + `get_convoy_mesh_context` (*lease* de 24 h, rotación al cambiar el roster) [L5]. Su roster solo incluye miembros con ESP32 registrado. Una futura RPC `get_convoy_voice_context` devolvería la época, una clave de voz `HKDF(group_key, "convlink-voice-v1")` y las claves públicas de los **teléfonos**.
4. **Invitaciones:** convLink genera `https://convlink.app/join/{CODE}`, pero **ninguna app lo abre** (no hay App Links, Associated Domains ni esquema propio). Oportunidad: enlaces universales compartidos (`/voice/{code}` o `convlink-voice://join?code=`) más un QR con el mismo código para unirse offline. La infraestructura ya existe: `SceneDelegate` → `appUrlOpen`; `src/hooks/useNavigationDeepLink.ts`.
5. **Coexistencia de audio:** convLink emite TTS de navegación con foco transitorio `MAY_DUCK`, y el proceso `:drive` también habla. mesh-app debe tolerar ese ducking sin dejar de capturar y no robar el foco de forma permanente.
6. **Coexistencia de radio:** convLink escanea NUS `6e400001-…` y nombres `^(convLink|CL)-[0-9a-f]{4}$`. mesh-app debe usar **sus propios UUID de 128 bits** y nunca anunciar nombres parecidos. Ambas apps comparten el controlador BLE (huecos de conexión y tiempo en aire).
7. **Identificadores:** convLink es `com.convlink.app` (equipo V6X9QP25Z6). mesh-app necesita un ID propio (p. ej. `com.convlink.voice`). Con el mismo equipo se podrían usar App Groups o un grupo de Keychain compartido.
8. **Alertas y posiciones:** si mesh-app retransmite datos cortos, reutilizar los tipos `ALT:` (P/H/A/R/C, IDs de 15 caracteres, expiración de 30 min). Las posiciones podrían ser una tercera fuente en `src/lib/convoyArbiter.ts`.

### 8.5 Trampas conocidas en convLink que aplican

- **iOS escaneo:** el plugin convierte la lista de servicios omitida en `[]` y no en `nil`. En iOS 27 solo un escaneo con servicio explícito encontraba el dispositivo.
- **iOS 27 nombres:** iOS 27 entregó anuncios sin nombre. Hay que poner flags + UUID de 128 bits + nombre corto en **un único anuncio primario** y nunca derivar identidad del UUID de periférico de iOS.
- **CCCD cifrado:** un CCCD cifrado lanza el diálogo de PIN dentro de `startNotifications`. Hubo que subir el timeout a 60 s. Un bond obsoleto aparece como "peer removed pairing information" (`src/ble/pairingRecovery.ts`).
- **Vitalidad:** debe depender de **cualquier** trama entrante. La versión basada en PING provocaba reconexiones en ~25 % de las reanudaciones. No comprobar estancamiento en background.
- **Generación de conexión:** cada callback necesita su generación, y la limpieza física debe terminar antes de reconectar.
- **Android GATT:** un Pixel 9 Pro XL mostró una desconexión remota inexplicada (status 19) tras el descubrimiento de servicios. Hay que contar con inestabilidad del GATT de Android.
- **Flags de permisos:** el manifiesto de convLink **no** declara `neverForLocation` en `BLUETOOTH_SCAN` (funciona solo porque tiene permiso de ubicación por el GPS). mesh-app debe declararlo y añadir `BLUETOOTH_ADVERTISE` y `NEARBY_WIFI_DEVICES`.
- **POST_NOTIFICATIONS:** en Android 13+, sin `POST_NOTIFICATIONS` concedido, la notificación del FGS se descarta en silencio.
- **Trabajo crítico:** nunca debe depender de temporizadores de la WebView. Se rechazaron los trucos `resumeTimers()` y `display:none`.
- **Sockets en background:** el SO los mata **sin** cambiar `navigator.onLine`. Volver a primer plano es la única señal fiable.
- **Build Android:**
  - R8/AGP 8.13.2 para metadatos de Kotlin 2.3.
  - Reglas `-keep` para las clases JNI (crítico para Opus/Oboe/Rust).
  - NDK fijado a 27.1.12297006 (por defecto se colaban ~454 MB de símbolos).
  - JDK 21.
- **Plugins de workspace:** se consumen desde `dist/`, así que hay que recompilar el plugin antes del type-check raíz.
- **`openAppSettings`:** llama a un plugin `NativeSettings` que no está instalado. Android no tiene forma de abrir los ajustes para recuperar permisos.
- **Documentación desactualizada:** las versiones del README y su §9 (protocolo v1) están obsoletas, y la memoria del usuario cita PocketBase. **El código manda.**

**No copiar:**
- Android Auto (`packages/convlink-auto`: CarAppService, proceso `:drive`, Valhalla, MapLibre Native) ni el stack de mapa y navegación.
- El protocolo v2 de ESP32/NUS como formato teléfono↔teléfono.
- `@capacitor-community/bluetooth-le` como transporte de malla.
- La estructura de `useBle.ts`.
- Trucos de keep-awake de la WebView.
- Identificadores y secretos (appId, keystore, `.env`, claves `comvoy-*`, nombres de la bóveda).
- `org.gradle.java.home` hardcodeado.
- Valores de plantilla obsoletos (`armv7`).
- El historial de 410 KB de BITACORA: empezar uno nuevo.
- Cualquier UI de chat de texto.

---

## 9. Riesgos críticos y bancos de prueba go/no-go (fase 0)

### 9.1 Riesgos críticos (ordenados)

| # | Riesgo | Impacto | Evidencia | De-riesgo |
|---|---|---|---|---|
| R1 | **Alcance real entre vehículos** muy inferior a la separación (20-500 m) → islas constantes | Mata la propuesta de valor | Sin mediciones; referencias informales de 10-150 m [NV] | T1. Si falla, reposicionar como "grupo compacto" o reconsiderar hardware |
| R2 | **iOS en background con casco:** un cambio de ruta (reconexión HFP) o una interrupción para la captura y no se recupera sin desbloquear | El piloto queda mudo sin saberlo | Reportes de 2019 [123]; iOS 26/27 sin verificar | T2; prototipos CallKit/PTT; notificación + Live Activity |
| R3 | **Descubrimiento con pantallas bloqueadas:** Android no ve un iPhone en background; dos iPhones bloqueados pueden no encontrarse | Reincorporación fallida tras una partición | Documentación de Apple [24][28]; estudio de 2020 [29] | T3; regla de roles; lobby; Live Activity (iOS 26) [31]; Aware emparejado |
| R4 | **Capacidad BLE con HFP activo** menor de lo estimado | Menos de 2 hablantes, cortes | Single-link ≠ teléfono↔teléfono; ~50 % de tiempo en aire [12][16] | T0 |
| R5 | **Wi-Fi Aware iPhone↔Android** no disponible en modelos comerciales | Grupos mixtos limitados a BLE | Sin data path público [48]-[52] | T4; lista blanca empírica por modelo |
| R6 | **Latencia de los cascos HFP** muy superior a 20-40 ms | 2-3 saltos >300 ms | Sin datos de Cardo/Sena [92][95] | T5 |
| R7 | **UX de PIN por par** en Wi-Fi Aware para 15 pilotos (14-105 emparejamientos) | Fricción, abandono | [37][43] | Pre-emparejar en el lobby solo un árbol de expansión; Aware opcional |
| R8 | **Android 17 / OEMs** matan o silencian el servicio | Mudez silenciosa | [143][152] | T2-A; reiniciar solo por la UI; Telecom; onboarding por OEM |
| R9 | **Rechazo en tiendas** (2.5.4, declaración de FGS) | Retraso del lanzamiento | [135][136][149] | Modo demo/loopback; vídeos; T7 |
| R10 | **Batería y temperatura** en rutas de 2-4 h | Teléfono apagado o en *throttling* | Sin datos primarios [157] | T6 |
| R11 | **Expectativa de "reemplazo de Cardo"** | Frustración | §1.2 | Posicionamiento honesto |
| R12 | Seguridad: reutilización de nonce, suplantación interna, rastreo | Compromiso | [190][197][200] | §5.5; fuzzing; contador persistido |
| R13 | Música degradada a mono mientras SCO está abierto | Rechazo de los pilotos | [67][68][100] | T5; informar; posible modo PTT con SCO intermitente (penaliza latencia) |

### 9.2 Bancos de prueba (hardware real)

Los umbrales son **propuestas iniciales para decidir con el usuario**, no datos.

| ID | Banco | Montaje | Métricas | Umbral go propuesto | Decide |
|---|---|---|---|---|---|
| **T0** | Throughput BLE iPhone↔Android | iPhone 12-17 + Android 14-17 (Pixel, Samsung, Xiaomi). L2CAP CoC inseguro en ambos sentidos de rol, GATT notify. **Con y sin casco HFP activo**. Medir el intervalo de conexión real | kbps sostenidos, pérdida, latencia por salto, intervalo negociado | ≥2 flujos Opus de 40 ms (~45 kbps de app) con pérdida <5 % y casco activo | S_max en BLE, FEC/DRED, rol central |
| **T1** | Alcance entre vehículos en movimiento | 2-3 vehículos; teléfono en soporte, bolsillo y bolsa de depósito; 50/100/200/300/500 m; recta, curvas, cuestas; BLE 1M/2M, Coded (Android), Wi-Fi Aware, hotspot | RSSI, pérdida, latencia, tiempo de reconexión frente a distancia | Separación útil ≥ la que el usuario defina como "convoy típico" (§11) | Viabilidad del producto |
| **T2** | Supervivencia en background | iOS 26/27: 4 h bloqueado con sesión de audio. Reconexión del casco, llamada entrante, Siri, otra app de audio. Android 17 (Pixel + Samsung + Xiaomi): FGS 4 h, muerte del proceso, `set-enable-hardening throw` | ¿La captura y la reproducción continúan? ¿Se recuperan solas? | 0 mudeces silenciosas; recuperación automática o con un toque guiado | Ancla de background (audio frente a CallKit/PTT/Telecom) |
| **T3** | Descubrimiento y reincorporación | Dos iPhones bloqueados; Android buscando un iPhone bloqueado (incluido el truco del bitmask del overflow); Live Activity iOS 26; partición y vuelta | Tiempo hasta reconectar | Reincorporación en un tiempo aceptable para el usuario (p. ej. ≤10 s) | Regla de roles, uso de Aware |
| **T4** | Wi-Fi Aware entre plataformas | iOS 26/27 con Galaxy S25/S26, Pixel 10/11 (Android 17), etc. Emparejamiento por PIN + NDP + throughput/latencia en modo realtime | ¿Funciona? Throughput, latencia, `maximumConnectableDevices`, `getNumberOfSupportedDataPaths()` | Funciona en ≥1 modelo popular → lista blanca | Alcance de la "subida" de transporte |
| **T5** | Cascos | Cardo Packtalk (Edge/Neo/Pro), Sena 50S/60S. Lazo con correlación cruzada (estilo TS 26.132) | Códec negociado (CVSD/mSBC), latencia por tramo, comportamiento AEC/NREC, música, botones, interferencia con su mesh propio | Latencia por tramo que mantenga el p95 <300 ms con 2 saltos | Presupuesto de jitter, modo PTT |
| **T6** | Batería y temperatura | 4 h de ruta real; pantalla apagada y con navegación; con y sin carga | %/h, estado térmico, *throttling* | Objetivo <5 %/h con pantalla apagada (propuesto) | Modo realtime, ciclos de escaneo |
| **T7** | Revisión | TestFlight + revisión interna con modo demo; declaración de FGS en Play (vídeos) | Aprobado/rechazado | Aprobación | Estrategia de modos declarados |
| **T8** | Rendimiento de UI | Pantalla de miembros/quién habla con 15 pares a 15 Hz + latencia del botón PTT en un Android de gama baja de referencia | Tiempo de frame, latencia de entrada | Sin jank perceptible; PTT <50 ms desde el toque (propuesto) | Capacitor frente a RN |
| **T9** | Simulador en CI (no es hardware) | 15 nodos, 20-500 m, ≤120 km/h, pérdida por distancia, churn | Latencia, pérdida, duplicados | Presupuestos de §3.3 | Regresiones del núcleo |

**Orden recomendado:** T0 y T1 primero (con un prototipo desechable de la opción C, un solo salto), después T2/T3/T5 en paralelo, y por último T4/T6/T7/T8. **No construir producto hasta que T0, T1 y T2 den "go".**

---

## 10. Afirmaciones refutadas o corregidas por la verificación (transparencia)

**Transportes**
1. "400-600 kbps es el agregado de un iPhone" → **Corregido:** es un único enlace periférico→iPhone contra un Nordic optimizado. Teléfono↔teléfono no está medido [12].
2. "bitchat es 100 % compatible" → **"fully protocol-compatible"**. Su Wi-Fi Aware es solo para Android y está desactivado por defecto desde el PR #712 [8][9].
3. **Contradicción entre verificadores:** el de transportes dijo que ninguna app de producción lleva voz en vivo multi-salto por BLE. El de producto confirmó en el código que bitchat tiene **live PTT desde julio de 2026** (PRs #1403/#1406, v1.7.0 en App Store), con tramas públicas retransmitidas. **Resolución:** bitchat sí lo tiene, solo en primer plano y sin medición independiente de la calidad multi-salto [183][184][185].
4. "Un Android no puede identificar un iPhone en background" → **Parcialmente refutado:** el overflow area se puede decodificar (bit en el bitmask de Apple), de forma probabilística y frágil [29]. Que el iPhone sea central sigue siendo lo robusto.
5. "−103 dBm es la sensibilidad Coded" → solo aplica a **Coded S=8 (125K)** [23].
6. "`isAwarePairingSupported()` comprueba la lista completa de Apple" → **Falso:** es un booleano reportado por el fabricante. Galaxy S25 da `true` y falla. Es necesario pero no suficiente [44][49].
7. "Pixel 9 y Xiaomi 14 no reportan soporte de emparejamiento" → el hilo solo dice que "no funcionaron" [48].
8. "`getSupportedOffloadBootstrappingMethods` (Android 37.2)" → **no aparece en AOSP main**. Descartado.
9. "El medio Aware de Nearby está excluido de `GITHUB_BUILD`" → **no confirmado**.
10. "Wi-Fi Aware tiene 4-5× el alcance de BLE" → se basa en **una sola prueba informal** en la que BLE dio <30 ft (un resultado anormalmente pobre). No está demostrado [53].
11. Cifras de alcance de Ditto (BLE ~130 m, AWDL ~35 m, Aware 100-300 m) → **no verificables**; las páginas actuales no tienen cifras [174]. FireChat: solo "about 30 feet" confirmado [175].
12. **Conflicto sobre Google Nearby en iOS:** el README dice "solo Wi-Fi LAN" [3]; la guía Swift menciona hotspot [4]; el repo tiene código de BLE/AWDL/Aware [2]. No es una dependencia fiable.
13. "El intervalo mínimo de 15 ms es un límite de iOS" → es una **guía para accesorios** (HID 11,25 ms; espaciales 7,5 ms). Se ha observado a iOS 26 usar 7,5 ms [17][18].

**Voz**
14. Tabla FEC original (16 kbps → NB a 30 %; 20 kbps → NB a 10 %, MB a 20-30 %) → **corregida por medición:** 16 kbps → MB a 30 %; 20 kbps → MB a 10 %, **WB** a 20-30 %. La histéresis no se había tenido en cuenta [78].
15. "OSCE no funciona con tramas de 40 ms" → **falso:** solo se salta 10 ms, NB y MB [81].
16. "LBRR con 40 ms cuesta +20 ms" → cuesta **+40 ms** (protege el paquete anterior de 40 ms).
17. "DRED es inasumible en BLE" → **falso con paquetes de 40 ms** (~4-7 kbps para 280-720 ms), siempre que se desactive LBRR [82].
18. "HFP limita siempre a 16 kHz" → con **LC3-SWB (HFP 1.9, Android 17) y LE Audio en Android** hay superbanda ancha. Elegir según la ruta de captura [68][70].
19. "SiLabs: HFP 20-30 ms" → **no verificable** (403). "Hilo 679274 de Apple: 193-260 ms" → **mal citado** (A2DP, `outputLatency` estable de 160,667 ms) [95].
20. "LC3 a 7,5 ms no da latencia proporcionalmente menor" y "LC3-SWB 50 Hz-14 kHz" → **no verificados**.
21. Las cifras de Opus de 26,5/46,5/66,5 ms no se publican asociadas a tamaños de trama. El mapeo es inferencia [87].
22. `VOZ_MESH_PLAN.md` fija 24 kbps → solo tiene sentido en Wi-Fi o cuando se quiere WB+FEC. En BLE hay que elegir 40 ms y 16 kbps o DRED.

**Políticas del SO**
23. "El único keep-alive de iOS es `audio`" → también sirve el modo **`location`** con actualizaciones continuas (sin arrancar el micro desde background) [122].
24. "Tras una interrupción no se puede reanudar la grabación" → desde iOS 13.1 **sí** se puede si ya se estaba grabando. El riesgo real es el **cambio de ruta** [123].
25. "DTS dijo que PTT no necesita push" → **malinterpretado:** DTS recomienda enviar push "every time" [128].
26. "Local Push Connectivity es solo Wi-Fi" → también LTE privada, Ethernet (iOS 26) y MCX (iOS 27) [129].
27. Las citas del hilo 674632 (CallKit) y del hilo 804414 (interop Android de Aware) eran **incorrectas**. La restricción real de interop está en el Apple ADG R31 [17].
28. "Apple sugirió añadir un interruptor de usuario" (hilo 671406) → **no respaldado**.
29. "Telecom delega WIU" → la página de Android 17 no lo dice explícitamente; solo que las apps VoIP con Telecom "unlikely to be impacted" [143].
30. "START_STICKY queda mudo en silencio" → un FGS de **micrófono** relanzado en background **lanza `SecurityException`**. El silencio afecta a un reinicio solo con `connectedDevice` y target 37.
31. La política de escaneo BLE de AOSP (10 min, 5/30 s) proviene del informe original y **no fue re-verificada**.
32. **Conflicto `.mixWithOthers`:** el informe de voz la recomienda; el de políticas y DTS la desaconsejan para background [101][128]. Queda pendiente de prueba (§3.5).
33. "`startLocalOnlyHotspotWithConfiguration` con SSID/PSK fijos es una API pública" → **disponibilidad para apps no-sistema sin confirmar**.

**Malla y producto**
34. "Powersports Business dice que Mesh Boost dobla de 15 a 31" → el artículo **no menciona 15 ni "double"**. TotalMotorcycle dice "more than doubles" [162][163].
35. "Remote Grouping por QR" → solo se describe un **enlace**. El QR no está verificado [163].
36. "Cardo pasó de 4 a 6 hablantes" → ambas cifras aparecen en la **1ª generación**. El tope tras Mesh Boost es **desconocido** [164][165].
37. Cifras de Cardo de BT 5.2 y ~13 h, y de Sena "6 hablantes" → **no verificadas** (webs de tiendas inaccesibles).
38. Gossip: "p 0,65-0,75 en grado 6" → p=0,65 en una malla regular de grado 6; 0,65-0,75 es para grafos aleatorios de grado medio 8 [194].
39. "El convoy tiene grado ≈2" → el grado es ≈2·⌊R/s⌋. Solo es una cadena pura cuando la separación se acerca al alcance.
40. bitchat: el recorte a TTL 5 depende del **grado local ≥6**, no del tamaño de la malla. La voz pública **sí** usa un subconjunto de enlaces. La "ausencia de auditoría" **no aparece en el whitepaper** [176][180][181].
41. "BLE es la única radio multiplataforma probada" → **matiz:** un iPhone unido a un hotspot Android es una vía Wi-Fi que funciona (en estrella). BLE es la única **peer-to-peer** [4].

**Framework**
42. El precedente de convLink es **más estrecho** de lo dicho: solo Android, FGS `location`/`dataSync`, sin audio, sin malla y sin motor headless en iOS (~350 LOC Swift) [L1].
43. "UniFFI siempre copia los argumentos" → **desactualizado**: desde 0.32.0, `&[u8]` va sin copia en funciones síncronas [230].
44. "El crate `opus` usa `audiopus_sys`" → ahora depende de **`opusic-sys`**; `audiopus_sys` no tiene mantenimiento desde 2021 [236].
45. munim-bluetooth 0.4.3 → es **0.10.2** y tiene más funciones. Sus timeouts de 15 s son valores por defecto [224].
46. L2CAP de "~100 KB/s con iPhone periférico" → medido contra un **ESP32-S3**, no contra un teléfono [14].
47. libopus 1.6 → apuntar a **1.6.1** (enero de 2026).
48. El simulador sans-IO **no** sustituye las pruebas de campo (no modela la pila BLE ni las políticas del SO).

---

## 11. Preguntas abiertas para el usuario (decisiones que cambian la arquitectura)

1. **Separación y alcance.** ¿Cuál es la separación "típica" que el producto debe cubrir sin cortes? Si es >100 m de forma sostenida, la evidencia sugiere que **solo teléfonos no basta**. ¿Es "solo teléfonos" una restricción absoluta, o se aceptaría en el futuro un relé opcional (p. ej. un nodo en la moto del líder o del cierre)?
2. **Experiencia de voz.** ¿Se acepta **PTT + VOX con tope de 1-2 hablantes en BLE**, en lugar de open-mic tipo Cardo? ¿Cuál es el modo por defecto: VOX (manos libres en moto) o PTT?
3. **Público.** ¿Motos con casco Bluetooth como caso principal, coches (altavoz/CarPlay/Android Auto) o ambos? Cambia AEC, ruteo y el modo por defecto.
4. **Plataformas mínimas.** ¿iOS 26+ / iPhone 12+ (necesario para Wi-Fi Aware) y Android 12+ (minSdk 31, evita la ubicación en background)? ¿Modo degradado solo BLE para dispositivos antiguos?
5. **Ritual previo a la ruta.** ¿Es aceptable un **"lobby" obligatorio** con todos los teléfonos desbloqueados para unirse por QR (y, si se usa Wi-Fi Aware, emparejar con PIN de iPhone a iPhone)? Determina si Aware entra en la v1.
6. **Hub hotspot.** ¿Es aceptable que un teléfono Android (líder o cierre, con carga) haga de hub hotspot en grupos mixtos, sabiendo que es un punto único de fallo?
7. **Relación con convLink.** ¿App separada, integración posterior o fusión en convLink? Afecta a la elección de Capacitor, a la identidad Supabase, al límite de 5 miembros del servidor, a las claves de grupo y a los enlaces de invitación.
8. **Fallback celular.** ¿Se permite usar internet cuando hay cobertura (como Sena WAVE), por ejemplo vía Supabase Realtime o WebRTC, para unir islas?
9. **Lenguaje del núcleo.** ¿Rust (recomendado), C++ (más familiar por el ESP32) o Swift en Android?
10. **Seguridad.** ¿Hace falta protección contra suplantación **dentro** del grupo (firmas por ráfaga) o basta con cifrado de grupo? ¿Y privacidad frente a rastreo (UUID rotatorio, v2)?
11. **Música.** ¿Es aceptable que la música de otras apps pase a mono/HFP mientras el intercom está activo? Si no lo es, ¿se aceptaría un modo PTT que libere SCO (más latencia al hablar)?
12. **Tamaño.** ¿El objetivo es 15, o hay que planificar 31 como Cardo?
13. **Botón PTT físico.** ¿Se contempla soportar mandos BLE de manillar o botones del casco (no verificado con Cardo/Sena)?
14. **Lanzamiento.** ¿App Store y Play a la vez? ¿Hay presupuesto y tiempo para solicitar entitlements (Wi-Fi Aware, posiblemente multicast) y para preparar los vídeos de declaración de FGS?
15. **Idioma de la documentación y el código.** convLink pasó a inglés en septiembre de 2026. ¿mesh-app sigue esa convención (código y docs en inglés, UI EN/ES) o usa español?

---

## 12. Fuentes

### A. Transportes (BLE, Wi-Fi Aware, hotspot)
1. https://github.com/google/nearby/discussions/2447
2. https://github.com/google/nearby/tree/main/internal/platform/implementation/apple
3. https://github.com/google/nearby/blob/main/connections/README.md
4. https://developers.google.com/nearby/connections/swift/get-started
5. https://github.com/google/nearby/blob/main/internal/platform/implementation/apple/Mediums/Aware/WiFiAwareMedium.swift
6. https://github.com/google/nearby/commits/main/internal/platform/implementation/apple/Mediums/Aware
7. https://raw.githubusercontent.com/google/nearby/main/connections/implementation/proto/offline_wire_formats.proto
8. https://cdn.jsdelivr.net/gh/permissionlesstech/bitchat-android@main/README.md
9. https://github.com/permissionlesstech/bitchat-android/pull/712
10. https://github.com/permissionlesstech/bitchat-android/pull/808
11. https://github.com/permissionlesstech/bitchat/pull/1704
12. https://developer.apple.com/forums/thread/770717
13. https://developer.apple.com/forums/thread/89644
14. https://developer.apple.com/forums/thread/723218
15. https://developer.apple.com/forums/thread/675960
16. https://punchthrough.com/ble-throughput-optimization-faq/
17. https://developer.apple.com/accessories/Accessory-Design-Guidelines.pdf
18. https://devzone.nordicsemi.com/f/nordic-q-a/124468/repeated-connection-interval-updates-on-ios-26/549156
19. https://developer.android.com/reference/android/bluetooth/BluetoothGatt
20. https://developer.android.com/reference/android/bluetooth/BluetoothAdapter
21. https://developer.apple.com/forums/thread/804458
22. https://novelbits.io/bluetooth-long-range-coded-phy/
23. https://www.bluetooth.com/learn-about-bluetooth/key-attributes/range/
24. https://developer.apple.com/documentation/corebluetooth/cbperipheralmanager/startadvertising(_:)
25. https://developer.apple.com/documentation/corebluetooth/cbcentralmanager/scanforperipherals(withservices:options:)
26. https://developer.apple.com/library/archive/documentation/NetworkingInternetWeb/Conceptual/CoreBluetooth_concepts/CoreBluetoothBackgroundProcessingForIOSApps/PerformingTasksWhileYourAppIsInTheBackground.html
27. https://developer.apple.com/forums/thread/814450
28. https://developer.apple.com/forums/thread/837516
29. http://www.davidgyoungtech.com/2020/05/07/hacking-the-overflow-area
30. https://github.com/davidgyoung/OverflowAreaBeaconRef
31. https://developer.apple.com/documentation/corebluetooth
32. https://developer.apple.com/forums/thread/766537
33. https://android.googlesource.com/platform/packages/modules/Bluetooth/+/refs/heads/main/system/internal_include/bt_target.h
34. https://developer.apple.com/documentation/wifiaware
35. https://developer.apple.com/documentation/wifiaware/adopting-wi-fi-aware
36. https://developer.apple.com/documentation/wifiaware/connecting-paired-devices
37. https://developer.apple.com/videos/play/wwdc2025/228/
38. https://developer.apple.com/documentation/wifiaware/washaredsecret
39. https://developer.apple.com/documentation/wifiaware/waperformancemode
40. https://developer.apple.com/documentation/wifiaware/wacapabilities
41. https://developer.apple.com/documentation/technotes/tn3213-moving-from-multipeer-connectivity-to-network-framework
42. https://digital-markets-act.ec.europa.eu/factsheet-how-dma-making-smartphones-better-interoperability-and-data-portability-case-studies-2026-05-11_en
43. https://developer.espressif.com/blog/2026/08/wifi-aware-esp-to-iphone/
44. https://developer.android.com/reference/android/net/wifi/aware/Characteristics
45. https://developer.android.com/reference/android/net/wifi/aware/AwareDataPathRequest
46. https://developer.android.com/develop/connectivity/wifi/wifi-aware
47. https://source.android.com/static/docs/core/connect/wifi-aware-architecture.pdf
48. https://developer.apple.com/forums/thread/790195
49. https://developer.apple.com/forums/thread/801280
50. https://developer.apple.com/forums/thread/797186
51. https://developer.apple.com/forums/thread/845528
52. https://hackmd.io/@grjte/cross-platform-wifi-aware
53. https://hackmd.io/@grjte/bitchat-wifi-aware
54. https://developer.apple.com/forums/thread/836279
55. https://developer.apple.com/forums/thread/785308
56. https://developer.apple.com/forums/thread/811828
57. https://developer.apple.com/forums/thread/805211
58. https://developer.apple.com/documentation/networkextension/nehotspotconfigurationmanager
59. https://developer.apple.com/documentation/networkextension/nehotspotconfiguration/joinonce
60. https://developer.apple.com/documentation/technotes/tn3179-understanding-local-network-privacy
61. https://developer.apple.com/documentation/bundleresources/entitlements/com.apple.developer.networking.multicast
62. https://developer.android.com/reference/android/net/wifi/WifiManager
63. https://developer.android.com/reference/android/net/wifi/SoftApConfiguration.Builder
64. https://developer.android.com/reference/android/net/wifi/p2p/WifiP2pConfig.Builder
65. https://blog.google/security/android-quick-share-support-for-airdrop-security/
66. https://github.com/dcavalcante/react-native-wifi-aware

### B. Motor de voz
67. https://developer.android.com/reference/android/media/AudioManager
68. https://source.android.com/docs/core/audio/sco-audio-mgmt
69. https://www.bluetooth.com/specifications/specs/hands-free-profile/
70. https://fuchsia.dev/reference/assembly/HfpConfig
71. https://supermegaultragroovy.com/2021/01/28/more-on-avaudioengine-airpods/
72. https://developer.apple.com/documentation/avfaudio/avaudiosession/categoryoptions-swift.struct/bluetoothhighqualityrecording
73. https://firmware.sena.com/senabluetoothmanager/UserGuide_50S_2.8.1_en_250731.pdf
74. https://opus-codec.org/demo/opus-1.5/
75. https://opus-codec.org/demo/opus-1.6/
76. https://opus-codec.org/release/stable/2025/12/15/libopus-1_6.html
77. https://opus-codec.org/downloads/
78. https://raw.githubusercontent.com/xiph/opus/v1.6.1/src/opus_encoder.c
79. https://raw.githubusercontent.com/xiph/opus/main/silk/enc_API.c
80. https://raw.githubusercontent.com/xiph/opus/main/silk/define.h
81. https://raw.githubusercontent.com/xiph/opus/main/dnn/osce.c
82. https://raw.githubusercontent.com/xiph/opus/main/dnn/dred_config.h
83. https://raw.githubusercontent.com/xiph/opus/main/configure.ac
84. https://datatracker.ietf.org/doc/draft-ietf-mlcodec-opus-dred/
85. https://www.rfc-editor.org/rfc/rfc6716.txt
86. https://opus-codec.org/release/stable/2023/04/20/libopus-1_4.html
87. https://opensource.googleblog.com/2022/09/lyra-v2-a-better-faster-and-more-versatile-speech-codec.html
88. https://github.com/drowe67/codec2
89. https://github.com/facebookresearch/encodec
90. https://www.cs.columbia.edu/%7Eandreaf/new/documents/other/T-REC-G.114-200305.pdf
91. https://itecspec.com/3gpp/22.179/s/annex-a
92. https://patents.google.com/patent/US10705793
93. https://docs.vivox.com/v5/general/core/5_19_0/en-us/Core/developer-guide/android/bluetooth-profile-comparison.htm
94. https://github.com/google/oboe/wiki/TechNote_BluetoothAudio
95. https://developer.apple.com/forums/thread/679274
96. https://developer.android.com/develop/connectivity/bluetooth/ble-audio/audio-manager
97. https://developer.apple.com/forums/thread/829759
98. https://www.bluetooth.com/blog/a-technical-overview-of-lc3/
99. https://developer.apple.com/documentation/avfaudio/avaudiosession/mode-swift.struct/voicechat
100. https://developer.apple.com/documentation/avfaudio/avaudiosession/categoryoptions-swift.struct/allowbluetoothhfp
101. https://developer.apple.com/documentation/avfaudio/avaudiosession/categoryoptions-swift.struct/mixwithothers
102. https://developer.apple.com/documentation/avfaudio/avaudioinputnode/voiceprocessingotheraudioduckingconfiguration
103. https://webrtc.googlesource.com/src/+/refs/heads/main/sdk/objc/components/audio/RTCAudioSessionConfiguration.m
104. https://developer.apple.com/forums/thread/769245
105. https://developer.apple.com/forums/thread/826462
106. https://raw.githubusercontent.com/google/oboe/main/include/oboe/Definitions.h
107. https://developer.android.com/games/sdk/oboe/low-latency-audio
108. https://developer.android.com/reference/android/media/audiofx/AcousticEchoCanceler
109. https://gitlab.linphone.org/BC/public/mediastreamer2/-/merge_requests/43
110. https://webrtc.googlesource.com/src/+/refs/heads/main/sdk/android/src/java/org/webrtc/audio/WebRtcAudioEffects.java
111. https://www.freedesktop.org/software/pulseaudio/webrtc-audio-processing/
112. https://github.com/xiph/rnnoise
113. https://github.com/Rikorose/DeepFilterNet
114. https://github.com/snakers4/silero-vad
115. https://webrtc.googlesource.com/src/+/HEAD/modules/audio_coding/neteq/g3doc/index.md
116. https://webrtc.googlesource.com/src/+/refs/branch-heads/4044/modules/audio_mixer/audio_mixer_impl.h
117. https://developer.android.com/media/optimize/audio-focus

### C. Políticas del SO, tiendas, batería
118. https://developer.apple.com/documentation/xcode/configuring-background-execution-modes
119. https://developer.apple.com/forums/thread/830728
120. https://developer.apple.com/forums/thread/685525
121. https://developer.apple.com/documentation/avfaudio/avaudiosession/category-swift.struct/record
122. https://developer.apple.com/documentation/corelocation/handling-location-updates-in-the-background
123. https://developer.apple.com/forums/thread/120038
124. https://developer.apple.com/documentation/avfaudio/handling-audio-interruptions
125. https://developer.apple.com/documentation/avfaudio/avaudiosession/setprefersnointerruptionsfromsystemalerts(_:)
126. https://developer.apple.com/documentation/pushtotalk/creating-a-push-to-talk-app
127. https://developer.apple.com/documentation/pushtotalk/ptchannelmanager/setaccessorybuttoneventsenabled(_:channeluuid:completionhandler:)
128. https://developer.apple.com/forums/thread/812592
129. https://developer.apple.com/documentation/networkextension/local-push-connectivity
130. https://developer.apple.com/documentation/livecommunicationkit
131. https://developer.apple.com/documentation/callkit/making-and-receiving-voip-calls
132. https://developer.apple.com/forums/thread/852293
133. https://developer.apple.com/forums/thread/835520
134. https://developer.apple.com/documentation/technotes/tn3115-bluetooth-state-restoration-app-relaunch-rules
135. https://developer.apple.com/app-store/review/guidelines/
136. https://developer.apple.com/forums/thread/826849
137. https://github.com/permissionlesstech/bitchat/blob/main/bitchat/Info.plist
138. https://github.com/permissionlesstech/bitchat/blob/main/bitchat/Features/voice/AudioSessionCoordinator.swift
139. https://support.apple.com/en-us/HT211876
140. https://developer.android.com/develop/background-work/services/fgs/service-types
141. https://developer.android.com/develop/background-work/services/fgs/restrictions-bg-start
142. https://developer.android.com/develop/background-work/services/fgs/changes
143. https://developer.android.com/about/versions/17/changes/bg-audio
144. https://developer.android.com/about/versions/17/behavior-changes-all
145. https://developer.android.com/about/versions/17/behavior-changes-17
146. https://developer.android.com/privacy-and-security/local-network-permission
147. https://developer.android.com/privacy-and-security/local-network-definition
148. https://support.google.com/googleplay/android-developer/answer/11926878
149. https://support.google.com/googleplay/android-developer/answer/13392821
150. https://developer.android.com/training/monitoring-device-state/doze-standby
151. https://android.googlesource.com/platform/frameworks/base/+/refs/heads/main/core/java/android/net/NetworkPolicyManager.java
152. https://dontkillmyapp.com/
153. https://android.googlesource.com/platform/packages/modules/Bluetooth/+/refs/heads/main/android/app/src/com/android/bluetooth/le_scan/ScanManager.java
154. https://android.googlesource.com/platform/packages/modules/Bluetooth/+/refs/heads/main/android/app/src/com/android/bluetooth/btservice/AdapterService.java
155. https://developer.android.com/develop/connectivity/bluetooth/bt-permissions
156. https://developer.android.com/develop/connectivity/wifi/wifi-permissions
157. https://source.android.com/docs/core/power/values
158. https://developer.android.com/develop/connectivity/telecom/voip-app/telecom
159. https://developer.android.com/about/versions/12/behavior-changes-all

### D. Producto, malla y seguridad
160. https://cardosystems.com/blogs/cardo-blog/dynamic-mesh-communication/
161. https://www.cardosystems.com/products/packtalk-pro/
162. https://powersportsbusiness.com/news/distributors-aftermarket/2026/08/27/new-cardo-update-expands-group-connectivity-for-packtalk-users/
163. https://www.totalmotorcycle.com/cardos-new-mesh-boost-expands-communication-to-31-riders-and-adds-new-public-mesh-and-remote-grouping-features/
164. https://ridermagazine.com/?p=45923
165. https://www.webbikeworld.com/motorcycle-intercoms/cardo-packtalk-review/
166. https://cardosystems.com/cdn/shop/files/Edge_Pocket-Guide_EN.pdf
167. https://www.sena.com/product/60s/
168. https://sena.com/en-us/?p=77048
169. https://community.sena.com/hc/en-us/articles/4415019612948-Using-the-Mesh-in-Open-Mesh
170. https://ultimatemotorcycling.com/2025/04/03/sena-60s-review-mesh-intercom-wave-intercom-and-more/
171. https://community.sena.com/hc/es/community/posts/41221518729492-60S-Fails-in-Mesh
172. https://www.ukgser.com/community/threads/any-real-difference-between-sena-and-cardo-built-in-comms-for-possible-new-shoie-helmet-and-can-they-talk-to-one-another.540400/post-6148672
173. https://www.bennetts.co.uk/bikesocial/reviews/products/motorcycle-technology/sena-60s-intercom-review
174. https://docs.ditto.live/sync/concepts/transports-overview
175. https://techcrunch.com/2014/04/03/firechat-brings-its-offline-chat-network-to-android/
176. https://github.com/permissionlesstech/bitchat/blob/main/WHITEPAPER.md
177. https://github.com/permissionlesstech/bitchat/blob/main/bitchat/Features/voice/PTTAudioFormat.swift
178. https://github.com/permissionlesstech/bitchat/blob/main/bitchat/Services/TransportConfig.swift
179. https://github.com/permissionlesstech/bitchat/blob/main/bitchat/Features/voice/PTTSettings.swift
180. https://github.com/permissionlesstech/bitchat/blob/main/bitchat/Services/RelayController.swift
181. https://github.com/permissionlesstech/bitchat/blob/main/bitchat/Services/BLE/BLEFanoutSelector.swift
182. https://github.com/permissionlesstech/bitchat-android/blob/main/docs/LIVE_VOICE_V1.md
183. https://github.com/permissionlesstech/bitchat/pull/1403
184. https://github.com/permissionlesstech/bitchat/pull/1406
185. https://apps.apple.com/us/app/bitchat-mesh/id6748219622
186. https://www.briarproject.org/news/2018-1.0-released-new-funding/
187. https://berty.tech/blog/bluetooth-low-energy/
188. https://techcrunch.com/2017/03/27/gotenna-pro-meshing-radio-aspires-to-deploy-next-to-rescue-fire-and-security-teams
189. https://en.wikipedia.org/wiki/Serval_Project
190. https://www.usenix.org/conference/usenixsecurity22/presentation/albrecht
191. https://eprint.iacr.org/2021/214
192. https://meshtastic.org/docs/overview/mesh-algo/
193. https://github.com/meshtastic/firmware/blob/master/src/mesh/PacketHistory.cpp
194. https://arxiv.org/abs/cs/0209011
195. https://personal.cis.strath.ac.uk/~terzis/classes/52.504/Broadcast_Storm.pdf
196. https://www.rfc-editor.org/rfc/rfc3626.html
197. https://www.rfc-editor.org/rfc/rfc9605.html
198. https://noiseprotocol.org/noise.html
199. https://www.rfc-editor.org/rfc/rfc9420.html
200. https://petsymposium.org/popets/2019/popets-2019-0036.php
201. https://www.rfc-editor.org/rfc/rfc8439

### E. Framework y núcleo compartido
202. https://ionic.io/blog/announcing-capacitor-8
203. https://capacitorjs.com/docs/updating/8-0
204. https://ionic.io/blog/the-road-to-capacitor-9
205. https://capacitorjs.com/docs/main/reference/support-policy
206. https://blog.cocoapods.org/CocoaPods-Specs-Repo/
207. https://reactnative.dev/blog/2025/10/08/react-native-0.82
208. https://docs.expo.dev/versions/latest/
209. https://expo.dev/changelog/sdk-56
210. https://nitro.margelo.com/docs/what-is-nitro
211. https://github.com/jhugman/uniffi-bindgen-react-native
212. https://docs.expo.dev/guides/dom-components/
213. https://registry.npmjs.org/nativewind
214. https://docs.flutter.dev/release/release-notes
215. https://api.flutter.dev/flutter/dart-ffi/NativeCallable/NativeCallable.listener.html
216. https://blog.jetbrains.com/kotlin/2025/05/compose-multiplatform-1-8-0-released-compose-multiplatform-for-ios-is-stable-and-production-ready/
217. https://kotlinlang.org/docs/native-memory-manager.html
218. https://kotlinlang.org/docs/native-objc-interop.html
219. https://dotnet.microsoft.com/en-us/platform/support/policy/maui
220. https://github.com/dotintent/react-native-ble-plx
221. https://pub.dev/packages/flutter_blue_plus
222. https://github.com/JuulLabs/kable
223. https://github.com/capacitor-community/bluetooth-le
224. https://cdn.jsdelivr.net/npm/munim-bluetooth@0.10.2/README.md
225. https://pub.dev/packages/bluetooth_low_energy
226. https://github.com/gary-quinn/kmp-ble
227. https://capawesome.io/docs/sdks/capacitor/bluetooth-low-energy/
228. https://crates.io/crates/uniffi
229. https://github.com/mozilla/uniffi-rs
230. https://raw.githubusercontent.com/mozilla/uniffi-rs/main/CHANGELOG.md
231. https://github.com/mozilla/uniffi-rs/issues/1974
232. https://github.com/matrix-org/matrix-rust-sdk/blob/main/bindings/matrix-sdk-ffi/Cargo.toml
233. https://github.com/boltffi/boltffi
234. https://crates.io/crates/rtrb
235. https://docs.rs/crate/opusic-sys/latest
236. https://crates.io/api/v1/crates/opus/0.4.0/dependencies
237. https://github.com/alta/swift-opus
238. https://github.com/element-hq/opus-swift
239. https://docs.rs/crate/sonora/latest
240. https://github.com/tonarino/webrtc-audio-processing
241. https://github.com/mcginty/snow
242. https://github.com/algesten/str0m
243. https://docs.rs/turmoil/latest/turmoil/
244. https://s2.dev/blog/dst
245. https://blog.bitdrift.io/post/optimizing-rust-mobile-sdk-binary-size
246. https://docs.rs/crate/ts-rs/latest
247. https://swift.org/documentation/cxx-interop/status/
248. https://www.swift.org/blog/swift-6.3-released/
249. https://www.swift.org/documentation/articles/swift-sdk-for-android-getting-started.html
250. https://developer.android.com/reference/android/net/wifi/aware/DiscoverySession

### F. Archivos locales (comvoy-app, solo lectura)
- L1. `/Users/ericksosagarcia/Desktop/personal/loRa/comvoy-app/docs/AA_HEADLESS_SESSION_PLAN.md`
- L2. `/Users/ericksosagarcia/Desktop/personal/loRa/comvoy-app/research/VOZ_MESH_PLAN.md`
- L3. `/Users/ericksosagarcia/Desktop/personal/loRa/comvoy-app/package.json`
- L4. `/Users/ericksosagarcia/Desktop/personal/loRa/comvoy-app/packages/convlink-auto/android/src/main/AndroidManifest.xml`
- L5. `/Users/ericksosagarcia/Desktop/personal/loRa/comvoy-app/supabase/migrations/20260914000000_convlink_mesh_security.sql`, `src/lib/meshSecurity.ts`, `docs/CONVLINK_SECURITY.md`
- L6. `/Users/ericksosagarcia/Desktop/personal/loRa/comvoy-app/src/lib/trustedOfflineIdentity.ts`, `src/lib/supabase/config.ts`, `src/lib/supabase/runtime.ts`
- L7. `/Users/ericksosagarcia/Desktop/personal/loRa/comvoy-app/supabase/migrations/20260910010000_convoy_lifecycle.sql`, `src/lib/tripService.ts`
- L8. `/Users/ericksosagarcia/Desktop/personal/loRa/comvoy-app/src/ble/` (`bleRequests.ts`, `convlinkProtocol.ts`, `discovery.ts`, `useBle.ts`, `pairingRecovery.ts`, `bleIdentityBoundary.ts`)
- L9. `/Users/ericksosagarcia/Desktop/personal/loRa/comvoy-app/tests/ios-scene-lifecycle.test.mjs`, `ios/App/App/SceneDelegate.swift`, `ios/App/App/Info.plist`
- L10. `/Users/ericksosagarcia/Desktop/personal/loRa/comvoy-app/src/i18n/`
- L11. `/Users/ericksosagarcia/Desktop/personal/loRa/comvoy-app/src/App.css`, `src/map/style/theme-presets.ts`, `components.json`
- L12. `/Users/ericksosagarcia/Desktop/personal/loRa/comvoy-app/android/app/src/main/java/com/convlink/app/ConvLinkVaultPlugin.java`, `ios/App/App/ConvLinkVaultPlugin.swift`
- L13. `/Users/ericksosagarcia/Desktop/personal/loRa/comvoy-app/scripts/tasks.mjs`, `BITACORA.md`, `AGENTS.md`
- [L-pitfalls] Entradas de `BITACORA.md` del 2026-09-18 ("Explicit iOS NUS discovery", "Cross-platform BLE discovery", "Allow human interaction during secure GATT subscription") y del 2026-07-25.

> **Limitaciones de esta investigación.** El presupuesto de búsqueda web de varias sesiones se agotó, así que parte de la re-verificación se hizo descargando directamente las URL primarias. No se pudieron re-comprobar varias páginas de tiendas (403/503), el benchmark de Infineon, las cifras antiguas de Ditto ni el detalle completo de algunos hilos de Apple. Todo lo marcado [NV] o [I] debe tratarse como hipótesis hasta validarlo en los bancos de prueba de §9.
