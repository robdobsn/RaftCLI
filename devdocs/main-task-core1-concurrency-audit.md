# Raft concurrency audit: moving the main task to core 1

Date: 2026-09-18
Status: investigation. The fixes subsequently made in the libraries (uncommitted, on `concurrency-hardening`
branches, not yet hardware tested) are recorded in `concurrency-hardening-changes.md`. File and line
references below are to the original commits listed in "Sources audited".

## 1. Purpose

A Raft project on an ESP32-S3 with a faulty WiFi antenna showed ~275 ms stalls of `loop()`. The
proposed mitigation is to scaffold new dual-core apps with `CONFIG_ESP_MAIN_TASK_AFFINITY_CPU1=y`
so the main task no longer shares core 0 with the WiFi, BT, `esp_timer`, `sys_evt` and Raft worker
tasks.

Pinning the main task to core 1 changes the concurrency model of every Raft app. Today nearly all
other tasks sit on core 0 at a higher priority than the main task. That gives an accidental
guarantee: **main-loop code never runs in the middle of another task's code unless that task
blocks**. With main on core 1 that guarantee disappears and main-loop code runs truly in parallel
with everything on core 0.

This document records an audit of the Raft libraries for concurrency problems of that kind, the
problems found that are already bugs today, and proposed solutions. It also restates the original
recommendations about the affinity change itself.

### Sources audited

Shallow clones of the `main` branches taken on 2026-09-18:

| Repo | Commit | Date |
|---|---|---|
| RaftCore | `feb4f77` "Added hook for code to run on bus task" | 2026-09-05 |
| RaftSysMods | `a1af7f1` "Note worker-task redesign in LoggerLoki devdoc" | 2026-08-12 |
| RaftWebServer | `0f04a21` "Bump version" | 2026-08-08 |
| RaftI2C | `0792b2f` "Added re-identify of devices" | 2026-09-05 |

File references below are written as `Repo/path:line` against those commits. Paths are shortened:
`RaftCore/core/...` means `RaftCore/components/core/...`, `RaftCore/comms/...` means
`RaftCore/components/comms/...`, `RaftSysMods/X/...` means `RaftSysMods/components/X/...`,
`RaftWebServer/...` means `RaftWebServer/components/RaftWebServer/...` and `RaftI2C/...` means
`RaftI2C/components/RaftI2C/...`.

### Method and confidence

Each library area was audited by reading the source. The High findings and a sample of the Medium
findings were then re-checked against the source independently; those are marked **[verified]**.
Statements about ESP-IDF, lwIP and NimBLE internals come from knowledge of those libraries, not
from reading their source, and are marked *(IDF behaviour, not read)* where they matter. Nothing
was run on hardware.

## 2. Summary

1. **The affinity change is still a sound default for dual-core chips, but it should not ship
   until a short list of fixes is made** (section 8, Phase 1). Seven findings are harmless or
   near-impossible today and become live bugs the moment main moves to core 1. The worst is a
   queue pattern that will duplicate inbound BLE data.
2. **The exposure is much smaller than first feared.** The initial assessment said web server REST
   handlers run on a web-server task and so race every SysMod's `loop()`. That is wrong. All HTTP,
   REST, WebSocket, RICREST and BLE command processing runs on the main loop. The only tasks that
   touch framework state from outside the main loop are: the `sys_evt` event handlers in
   `NetworkSystem`, the NimBLE host task, the I2C worker, the OTA task, the Loki log worker, the
   socket listener, and any task that calls `LOG_x`.
3. **Most of the serious findings are already bugs today** and should be fixed regardless of the
   affinity decision. The main ones: I2C hardware driven from two tasks with no bus lock; lost
   device status notifications in `BusStatusMgr`; WiFi stop/start executed inside the NimBLE host
   task on every BLE connect when `pauseWiFiforBLE` is set (the scaffold default); unlocked lwIP
   raw-API calls for DNS and NetBIOS; a logger ring buffer freed while other tasks write to it;
   `NetworkSystem` Strings written by `sys_evt` while the main loop copies them.
4. **The affinity change only addresses one cause of `loop()` stalls** (preemption by core-0
   tasks). Section 7 lists blocking calls made from the main loop itself, which pinning does not
   help. One of them is a verified functional bug that stalls the loop for 500 ms per I2C device
   command.

Counts: 9 High (2 of them conditional on configuration), about 20 Medium, about 30 Low.

## 3. Threading model as found

### 3.1 Tasks

| Task | Priority | Core | What it runs in Raft code |
|---|---|---|---|
| main (`app_main`) | 1 | 0 today | `RaftCoreApp::loop()` -> `SysManager::loop()` -> every SysMod `loop()`, then `delay(1)` |
| IDF WiFi | 23 | 0 | none directly |
| IDF `esp_timer` | 22 | 0 | none found |
| IDF `sys_evt` | 20 | 0 | `NetworkSystem::wifiEventHandler`, `ipEventHandler`, `ethEventHandler` |
| lwIP tcpip | 18 | none | SNTP sync callback, `DNSResolver::dnsResultCallback` |
| NimBLE host | high | per `CONFIG_BT_NIMBLE_PINNED_TO_CORE` (default 0) | GAP event handler, GATT access callbacks, advertising restart, BTHome advert decode |
| `socketLstnTask` | 9 | **unpinned** (see WS2) | `accept()` then queue hand-off only |
| BusI2C worker | 5 | `taskCore` (default 0, often 1) | scan, identify, poll, queued requests, bus-task hook |
| `OTATask` | 5 | 0 | flash writes |
| `LokiLog` | 1 | unpinned | blocking HTTP log upload |
| `BLEOutQ` | 1 | 0 | only if BLE `taskEnable=1` (default off) |
| `SDUsedScan` | 5 | unpinned | one-shot FAT free-space scan |
| any task calling `LOG_x` | - | - | `LoggerCore::log()` -> every registered logger's `log()` |

### 3.2 What runs on the main loop (correction to the initial assessment)

All of the following run on the main task, so they do **not** race SysMod `loop()` code:

- **HTTP and REST.** `USE_THREAD_FOR_CLIENT_CONN_SERVICING` is commented out
  (`RaftWebServer/RaftWebConnManager.cpp:22`). Connection servicing runs from
  `RaftWebConnManager::loop()` (`:93-98`). Endpoint callbacks are invoked from
  `RaftWebResponderRestAPI.cpp:151, 273` under `RaftWebConnection::loop()`. **[verified]**
- **WebSocket inbound and outbound.** Socket `recv` and `send` happen in the main loop.
- **All comms-channel inbound processing.** `COMMS_CHANNEL_USE_INBOUND_QUEUE` is defined
  (`RaftCore/comms/CommsChannels/CommsChannel.h:20`). `CommsChannel::handleRxData` only queues the
  bytes. Codec decode, `ProtocolExchange` and `RestAPIEndpointManager::handleApiRequest` run from
  `CommsChannelManager::loop()`. This holds for BLE too: the NimBLE host task only does
  `_inboundQueue.put()`. **[verified]**
- **Outbound messages.** Every `outboundHandleMsg` caller in the four repos is on the main loop
  (`StatePublisher`, `ProtocolExchange`, file protocols, `SysManager::sendReportMessage`).
- **Status JSON, publish generation, `NetworkManager` status callbacks** (dispatched from its
  `loop()`), `RaftSystemTime::notifyChanged("sntp")` (deferred to `NetworkSystem::loop()`).

So `SysManager` API handlers, `ProtocolExchange` sessions, codecs, `StatePublisher`, `FileManager`,
`SerialConsole`, MQTT, `LEDPixels` and the `devman/*` handlers are single-task today.

### 3.3 Which task runs user-visible callbacks

App authors will assume main-loop context. These do not have it:

| Callback | Runs on |
|---|---|
| `RaftDeviceDataChangeCB` (`DeviceManager::registerForDeviceData`, bus devices) | **I2C worker** |
| `BusRequestCallbackType` for poll requests | **I2C worker**, with `_pollingMutex` held |
| `BusRequestCallbackType` for non-poll requests | main loop (`BusAccessor::loop`) |
| `VirtualPinSetCallbackType` on an IO expander pin | **I2C worker** |
| `RaftNewDeviceIdentFn`, `RaftBusTaskServiceFn` (bus-task hook) | **I2C worker** |
| SysMod status-change callbacks from `BLEManager` | **NimBLE host task** (finding B1) |
| BTHome data-change and bus element status callbacks | **NimBLE host task** (finding B7) |
| `RaftDeviceStatusChangeCB`, `BusElemStatusCB`, `BusOperationStatusCB` (I2C) | main loop |
| Publish message generators, state detection | main loop |
| SysMod status-change callbacks from `NetworkManager` | main loop |

## 4. Reading the findings

Each finding states whether it is a bug **today** (main on core 0) or only **becomes** one when
main moves to core 1.

- *Today*: a higher-priority task on core 0 can preempt main at any instruction. So any case where
  main is mid-read or mid-update and the other task writes is already reachable. Unpinned tasks and
  `taskCore:1` I2C configs already run in parallel with main.
- *Becomes*: cases that need main to run while the other task is mid-update. On one core that
  needs the other task to block inside the critical region, which usually cannot happen.

Severity: **High** means memory corruption, a crash, or persistent functional failure on a likely
path. **Medium** means the same on an unlikely path, or a recoverable functional failure. **Low**
means wrong statistics, cosmetic effects, or latent code.

## 5. Findings that become bugs when main moves to core 1

These are the blockers for the affinity change.

### Q1. Inbound queue consumer ignores a failed `get()` - High, becomes [verified]

- Where: `RaftCore/comms/CommsChannels/CommsChannel.cpp:164-192`,
  `RaftCore/core/ThreadSafeQueue/ThreadSafeQueue.h:59-80`.
- `processInboundQueue()` does `peek(msg)`, feeds the bytes to the codec, then calls
  `_inboundQueue.get(msg)` with the default 0 ms timeout and ignores the result. `get()` is a
  try-lock. If the producer holds the queue mutex at that moment, the message stays queued and is
  fed to the codec again on the next loop.
- Producers off the main loop: the NimBLE host task (`BLEGapServer.cpp:683`), and the AsyncTCP task
  if `CommandSocket` is ever enabled.
- Today this cannot happen: the producer has higher priority on the same core and never blocks
  while holding the mutex. With main on core 1 it is a routine race on every inbound BLE packet.
- Effect: duplicated HDLC bytes (CRC failure, dropped frame) or a whole frame delivered twice
  (command executed twice, duplicated file or OTA block). Worst during BLE file upload and OTA.
- Fix: check `readyForRxData()` first, then do one `get(msg, timeoutMs)` and process only if it
  returned true. There is a single consumer so the peek is unnecessary.

### Q2. `ThreadSafeQueue` defaults make contention look like "empty" - Medium, becomes

- Where: `RaftCore/core/ThreadSafeQueue/ThreadSafeQueue.h`.
- Every method defaults to a 0 ms try-lock. `count()` returns 0 on lock failure. `clear()` silently
  does nothing. `canAcceptData()` reads `_queue.size()` with no lock. An unused
  `DEFAULT_MAX_MS_TO_WAIT = 1` constant suggests a non-zero default was intended.
- Callers that ignore results: `CommsChannel.cpp:190` (Q1), `CommsChannel::outboundQueueAdd`
  (`:207`), `BLEGattOutbound.cpp:311-315` (B2), `BusAccessor::clear`.
- Also: `_maxLen` is `uint16_t` while the setters take `uint32_t`; mutex creation is never
  checked; the class is copyable, which would double-delete the semaphore.
- Fix: a short blocking default (5-10 ms, or wait forever, since the critical sections are tiny);
  make `count()` report failure distinctly; mark results `[[nodiscard]]`; delete the copy
  constructor; add a combined pop-if operation.

### Q3. Protocol codecs are created lazily from whichever task gets there first - Medium, becomes

- Where: `RaftCore/comms/CommsChannels/CommsChannelManager.cpp:583-628`, called from
  `inboundHandleMsg` (`:322`, NimBLE host task) and from main (`:104, 286, 388, 553`).
- Unsynchronised check-then-create. Two codecs can be created; one leaks with its buffers (up to
  ~200 kB with PSRAM); partial-frame state is lost if main was already using the first. Codec
  construction on the NimBLE host stack also parses config (finding C1) and logs.
- Fix: create codecs on the main loop only (in `registerChannel`/`addProtocol` or the first
  `loop()`). `inboundHandleMsg` does not need the codec because it only queues.

### B4. BLE connection state is a non-atomic pair, and flags are set before their timestamps - Medium, becomes

- Where: `RaftSysMods/BLEManager/BLEGapServer.cpp:730-748, 981-985, 196-197`,
  `BLEGattServer.h:69-73`.
- `_isConnected` is written before `_bleGapConnHandle`. A reader on the other core sees
  connected=true with handle 0; NimBLE returns `ENOTCONN`; `sendToCentral` maps that to FAIL and
  the first response after connect is silently dropped.
- `gapEventConnect` sets `_connIntervalCheckPending = true` before `_connIntervalCheckPendingStartMs`.
  A parallel main loop sees pending with a stale time, fires immediately on handle 0, fails, and
  clears the flag. The preferred connection interval is then never requested, so the link stays at
  the central's default (lower throughput).
- The same flag-before-timestamp ordering exists in `BLEGapServer::restart()`
  (`_bleRestartState` then `_bleRestartLastMs`) and in `NetworkSystem` (finding N4).
- Also: 0 is used as the "no handle" sentinel but is a valid NimBLE handle.
- Fix: one `std::atomic<uint16_t>` handle with `BLE_HS_CONN_HANDLE_NONE` as the sentinel,
  snapshotted once per function. Always write the timestamp before the flag. Better, move all of
  this to main-loop edge handling (see B1).

### N4. mDNS deferral flag set before its timestamp - Low/Medium, becomes

- Where: `RaftCore/core/NetworkSystem/NetworkSystem.cpp:1354-1355, 1386-1387, 1401-1402` vs
  `:235-239`.
- `sys_evt` sets `_mdnsSetupPending = true` then `_mdnsSetupPendingMs = millis()`. Main on the
  other core can see the flag with the old timestamp, so `setupMDNS()` runs immediately instead of
  after the 500 ms settle delay that the code comments say is needed.
- Fix: write the timestamp first; make the flag atomic.

### I3. I2C interrupt lands on main's core; timeout path is only safe when ISR and worker share a core - Medium/High, becomes (for default configs)

- Where: `RaftI2C/I2CCentral/RaftI2CCentral.cpp:892-914` (`esp_intr_alloc_intrstatus`, called from
  `BusI2C::setup` on the **main task**), `:445-494`, `:538-543`, `:1135-1171`.
- ESP-IDF allocates an interrupt on the core of the calling task. Today that is core 0, the same
  core as a default (`taskCore:0`) worker. After the change the ISR runs on core 1 and the worker
  on core 0.
- On a software timeout the task runs `emptyRxFifo()` and then sets `_readBufStartPtr = nullptr`
  (`:542`) without the spinlock, with interrupts still enabled. The ISR's null check (`:1138`) is
  outside the critical section (`:1142`). Cross-core, the ISR can pass the check and then write
  through a null pointer or into the caller's released stack buffer. **[verified structurally]**
- This already applies today to configs that set `"taskCore": 1`.
- Related and already a bug today: there is no drain of `_accessSemaphore` before starting a
  transaction, so a late ISR after a timeout leaves a token and the next `access()` returns
  immediately. This is documented in `RaftI2C/devdocs/i2c-bus-speed-change-investigation.md`; the
  fixes were tried and reverted.
- Fix: allocate the interrupt on the worker's core (do `init()` from the worker task, or use
  `esp_ipc_call`). On any timeout, disable the interrupts and null the buffer pointers inside the
  `_i2cAccessMutex` critical section, and move the ISR's null check inside it. Drain the semaphore
  with a zero-timeout take before `trans_start`.

### O1. OTA status mutex uses 1-tick takes and ignores failure - Low/Medium, changes

- Where: `RaftSysMods/ESPOTAUpdate/ESPOTAUpdate.cpp:81, 143, 388, 419, 441-442, 506`.
- Today the main side can never find the mutex held. After the change `apiFirmwareMain`, which
  reports the final OTA result, can find it held by the worker and then reports failure for a
  successful update. Today's worker-side failure (a spurious "FailedStartOTA" when main holds the
  lock for more than 1 ms) gets better.
- Fix: wait forever or at least 100 ms; never fail the OTA because of the statistics lock; move the
  CRC computation outside the lock.

## 6. Findings that are already bugs today

Fix these regardless of the affinity decision. The change makes most of them more likely.

### 6.1 NetworkSystem

**N1. Strings written by `sys_evt` and read by the main loop with no lock - Medium/High [verified]**

- Where: `RaftCore/core/NetworkSystem/NetworkSystem.cpp`. Writers on `sys_evt`: `_wifiStaSSID`
  (`:1199, 1431`), `_wifiIPV4Addr` (`:1346, 1361, 1430`), `_ethIPV4Addr` (`:1381, 1392`),
  `_ethMACAddress` (`:1306, 1314`). Readers on main: `getConnStateJSON` (`:344-403`),
  `setupMDNS` (`:1477`), the copy getters in `NetworkSystem.h:70-99`. Main-loop writers:
  `clearCredentials` (`:731-733`), `:535`, `:666`.
- The `String` class keeps at most 14 characters inline, so a 15-character IP, any MAC string and
  longer SSIDs are heap-backed. `sys_evt` at priority 20 can preempt main mid-copy and free the
  buffer it is copying from. After the change there is also a write/write race between `clear()` on
  main and an assignment on `sys_evt`.
- Effect: garbage in status JSON, or a rare `LoadProhibited` crash, typically around WiFi
  connect/disconnect, which is when both sides are active.
- Fix: event handlers should not touch Strings. Post the event data to a small FreeRTOS queue (or
  fixed `char` buffers guarded by a spinlock) and apply it in `NetworkSystem::loop()`. This is the
  pattern already used for SNTP (`_sntpSyncPendingNotify`).

**N2. `pauseWiFi()` sets `_isPaused` after `stopWifi()` - Medium [verified]**

- Where: `NetworkSystem.cpp:748-783`, `:1415-1435`, `:1360-1361`.
- `esp_wifi_stop()` raises `STA_DISCONNECTED`, handled on `sys_evt` while `_isPaused` is still
  false. `handleWiFiStaDisconnectEvent` then calls `esp_wifi_connect()` on a stopping driver and
  clears `_wifiIPV4Addr` and `_wifiStaSSID`, which the pause logic is written to preserve
  (`getConnStateJSON(..., useBeforePauseValue)`).
- Fix: set a pausing flag before `stopWifi()`; make it atomic.

**N3. WiFi control operations have no mutual exclusion - High via B1**

- `pauseWiFi`, `configWifiSTA`, `configWifiAP`, `clearCredentials`, `wifiScan`, `setHostname`,
  `startWifi` and `stopWifi` mutate `_isPaused`, the three event-handler instance pointers, the
  netif pointers, `_numWifiConnectRetries` and `_wifiStaSSIDConnectingTo`. They are reached from
  the main loop and, through finding B1, from the NimBLE host task, concurrently with
  `NetworkSystem::loop()` calling `esp_wifi_sta_get_ap_info`.
- Fix: fix B1 so all control operations run on the main loop. Add a debug assert on task identity.

**N5. lwIP raw API called from the main task with no core lock - Medium**

- Where: `NetworkSystem.cpp:1521-1524` (`netbiosns_init`, `netbiosns_set_name` in `setupMDNS`),
  `:854` (`setHostname`), `:824-834` (`esp_netif_next_unsafe` iteration).
- `netbiosns_init` creates and binds a UDP pcb. lwIP raw-API functions must run in the tcpip thread
  or under `LOCK_TCPIP_CORE()` *(IDF behaviour, not read)*. The tcpip thread is unpinned at
  priority 18, so this can corrupt the pcb list today. Same class as finding D1.
- Fix: wrap in `esp_netif_tcpip_exec()` or `tcpip_callback()`. Replace the `_unsafe` iteration with
  `esp_netif_find_if` or run it in tcpip context.

**N6. `WiFiScanner::_scanInProgress` - Low**

- Where: `RaftCore/core/NetworkSystem/WiFiScanner.cpp:34-47`. Plain bool set on main and cleared on
  `sys_evt`. It is set before `esp_wifi_scan_start` and never reset if that call fails, or if WiFi
  is paused mid-scan, so results are then blocked forever.
- Fix: atomic flag; reset on failure and on pause.

**N7. Other plain shared scalars - Low**

- `_numWifiConnectRetries` (incremented on `sys_evt`, zeroed on main), the
  `_pendingWiFiDisconnectWarn`/`_pendingWiFiDisconnectWarnRetries` pair, `_wifiAPClientCount`,
  `_wifiRSSI`. Single-word writes; lost updates only. Make them atomic.

**N8. Blocking calls made from `NetworkSystem::loop()` and its API handlers - see section 7.**

Checked and safe: the connection-state bits use a FreeRTOS event group; `RaftSystemTime` uses a
mutex with a snapshot taken under the lock and callbacks made outside it; the SNTP callback only
sets a flag and logs with `ESP_LOGI`.

### 6.2 BLEManager

**B1. BLE connect/disconnect runs WiFi stop/start inside the NimBLE host task - High [verified]**

- Chain: `BLEGapServer::setConnState` (`RaftSysMods/BLEManager/BLEGapServer.cpp:746-747`) ->
  `BLEManager` lambda (`BLEManager.cpp:32-34`) -> `RaftSysMod::executeStatusChangeCBs`
  (`RaftCore/core/SysMod/RaftSysMod.cpp:255-262`) -> `SysManager::statusChangeBLEConnCB`
  (`SysManager.cpp:1561-1571`) -> `NetworkSystem::pauseWiFi`.
- Active when `pauseWiFiforBLE` is 1. **The RaftCLI scaffold sets this to 1**
  (`raft_templates/systypes/{{sys_type_name}}/SysTypes.json`), so every scaffolded BLE app has it.
- The host task runs `esp_wifi_stop`/`deinit`/`init`/`start`, handler registration, String
  assignment and logging inside the GAP CONNECT event, before MTU exchange and connection
  parameter handling can proceed. It mutates the state listed in N3 with no lock while the main
  loop runs `NetworkSystem::loop()`. NimBLE host stack headroom for `esp_wifi_init` was not
  checked.
- Also `_statusChangeCBs` is a `std::list` appended on main during `postSetup` after the host task
  has started (boot-window race, Low).
- Fix: `setConnState` stores the new state in an atomic. `BLEGapServer::loop()` on the main loop
  detects the edge and calls `_statusChangeFn`. This matches `NetworkManager::loop()`
  (`NetworkManager.cpp:92-105`), which already does it correctly.

**B2. BLE outbound queue: duplicate sends and dropped messages - High when `taskEnable=1`, otherwise not applicable**

- Where: `BLEGattOutbound.cpp:311-315`, `:170, 179`, `:133, 144`.
- `if (removeFromQueue) { queue.get(bleOutMsg); msgPos = 0; }` ignores a failed `get()` but still
  resets `msgPos`, so the whole message is sent again. `sendMsg` uses `put()` with a 0 ms timeout,
  so a response is dropped whenever the outbound task holds the mutex (it copies a whole message
  under the lock in `peek`). Already possible today by time-slicing (both priority 1); much worse
  in parallel.
- Fix: non-zero timeouts; treat the message as removed only if `get()` succeeded; better, pop the
  message into a member and chunk from that.

**B3. Advertising is started from two tasks and reads config on the host task - Medium**

- Where: `BLEGapServer.cpp:370` called from the host task (`:349, 544, 999`) and from main
  (`:1227`); `BLEManager::getAdvertisingInfo` (`BLEManager.cpp:328-348`).
- `getAdvertisingInfo` reads `RaftJson` config and `SysManager` Strings on the host task (finding
  C1). The two `startAdvertising` callers can interleave `ble_gap_adv_set_fields`,
  `ble_svc_gap_device_name_set` and `ble_gap_adv_start`, producing `EALREADY` errors or advertising
  data from one call paired with the scan response from the other.
- Fix: start advertising from the main loop only; host events set a flag. Or cache the name,
  manufacturer data and serial on the main loop and use only the cache on the host task.

**B5. BLE restart sequence - Medium; part (d) is a use-after-free regardless of threading**

- Where: `BLEGapServer.cpp:203-211, 836-855, 756-832, 1166-1203`, `BLEGattServer.cpp:415-502`.
- (a) `nimbleStop()` runs `nimble_port_stop()` and `nimble_port_deinit()` on main without
  quiescing the `BLEOutQ` task, which may be inside `ble_gatts_notify_custom`.
- (b) `getStatusJSON` calls `ble_gap_*_active()` behind a non-atomic host-ready check.
- (c) `nimbleStop()` failure is ignored, so `nimble_port_init` can be called twice.
- (d) `nimbleStart()` calls `_gattServer.start()` again, which pushes again into
  `_mainServiceCharList`, `_servicesList` and `_stdServices` (`BLEGattServer.cpp:422-479`)
  **[verified]**. The first service entry keeps `.characteristics = _mainServiceCharList.data()`
  from before the vector reallocated, so NimBLE is handed a dangling pointer and duplicated tables.
- Fix: build the service tables once or clear them before rebuilding; request restart through one
  atomic enum with the main loop stamping the time; gate every off-host NimBLE call with an atomic
  host-ready flag; quiesce the outbound task before stopping; check return codes.

**B6. In-flight indication counter - Low/Medium.** `BLEGattOutbound.cpp:208-224, 264-272, 300-307,
383-396`. 2 ms mutex takes whose failure silently skips the increment or decrement;
`isIndicationInFlight()` writes 0 without the mutex. Fix: `std::atomic<int32_t>` with
compare-exchange, plus a sequence number so a stale ACK cannot decrement a newer indication.

**B7. BTHome/central path runs DeviceManager and user callbacks on the host task - Medium if BLE
central is used.** `BLEBusDeviceManager.cpp:194-295` -> `RaftBus::callBusElemStatusCB` ->
`DeviceManager::busElemStatusCB` (`DeviceManager.cpp:208-309`), which iterates
`_requestedDeviceDataChangeCBList` while the main loop mutates it with no lock (`:1675-1723`).
This is what makes finding DM1 reachable. Fix: the host task only stores decoded adverts; status
and data callbacks are raised from the main loop.

**Low items.**

| ID | Where | Issue | Fix |
|---|---|---|---|
| B8 | `BLEManStats.h`, `RaftCore/core/NumericalFilters/MovingRate.h:38-72` | stats written on host and outbound tasks, read on main; possible transient out-of-range index | short critical section or atomics |
| B9 | `BLEStdServices.cpp:93`, `.h:84-92` | `double` written on main, read on host (torn read) | store pre-formatted bytes atomically |
| B10 | `BLEGapServer` various | plain shared scalars: `_advertisingCheckRequired`, `_responseNotifyState`, `_actualMtuSize`, `_rssi` (written as 0 then the value), `_bleConfig.connIntervalPreferredBLEUnits` | atomics; write RSSI via a local |
| B11 | `BLEGapServer.cpp:116-127` | `setConnState(false)` called after `nimbleStart()`; can overwrite a real connect at boot | call it first |
| B12 | `BLEGattOutbound.cpp:97-105` | `vTaskDelete` of a task that may hold a mutex or the NimBLE host lock; cooperative exit is never signalled | notify, wait for exit, then stop the stack |
| B13 | `BLEGattOutbound.cpp:359` | task busy-spins if `minMsBetweenSends` is 0 or the tick rate is below 1 kHz | clamp to at least 1 tick |
| B14 | `BLEGapServer.cpp:269, 283, 457` | `ble_svc_gap_device_name()` static buffer read while being rewritten | fixed by B3 |

### 6.3 Configuration (RaftJson)

**C1. Config documents are replaced in place while the NimBLE task parses them - Medium [verified]**

- Where: `RaftCore/core/RaftJson/RaftJson.h:839-865` (`setSourceStr`), `RaftJsonNVS.cpp:43-63,
  227-257`, `SysManager.cpp:1496-1555`, `SysTypeManager.cpp:223-232, 463-524`.
- `setSourceStr` move-assigns `_jsonStr` (freeing the old buffer), then updates `_pSourceStr`, then
  `_pSourceEnd`. There is no lock, and all getters parse directly from those pointers.
- Writers are all on main: `setFriendlyName`, `apiSerialNumber`, `postsettings`, `clearsettings`,
  `configSaveData`. The off-main readers found are `BLEManager::getAdvertisingInfo` and lazy codec
  construction (Q3), both on the NimBLE host task.
- Today the window is a few instructions. With main on core 1 it is the whole host-side parse.
  Trigger: a settings or friendly-name write while a BLE central disconnects and the device
  re-advertises. Effect: garbage advertising name, or the parser runs off a freed buffer.
- `SysTypeManager::selectBest()` also nulls the chained document and re-sets it (`:483-521`), so a
  concurrent reader sees defaults or a mismatched start/end pair.
- Fix (preferred): remove the off-main readers by fixing B3 and Q3, and document "RaftJson is
  main-loop only unless you copy values during `setup()`". Fix (general): hold the document as a
  `std::shared_ptr<const std::vector<char>>` swapped under a mutex, with getters taking a local
  copy of the pointer before parsing.

**C2. Lazy static caches - Low.** `getSystemMACAddressStr` uses static `String` caches
(`RaftCore/core/Utils/PlatformUtils.cpp:90-125`), currently called only on main; the separator
toggling also defeats the cache. `NamedValueProvider::getNullProvider()` is a lazy `new`. Fix:
fixed `char[18]` buffers computed once; a function-local static.

### 6.4 DNS, loggers and OTA

**D1. `dns_gethostbyname` called outside the tcpip thread with no core lock - High [verified]**

- Where: `RaftCore/core/DNSResolver/DNSResolver.cpp:57-58`. Callers: `LoggerPapertrail.cpp:153`
  and `RaftMQTTClient.cpp:358` (main loop), `LoggerLoki.cpp:338` (unpinned worker).
- This is lwIP's raw API. It enqueues and sends the query inline, touching the DNS table, UDP pcbs
  and pbuf pools owned by the tcpip thread. There is no `LOCK_TCPIP_CORE()` or `tcpip_callback`
  anywhere in these files. The Loki worker can already run in parallel with tcpip today.
- Effect: sporadic lwIP asserts or crashes; lost DNS replies, which then trigger D2.
- Fix: run the lookup in tcpip context (`tcpip_callback`, `esp_netif_tcpip_exec`), or use
  `getaddrinfo()` from a worker. Enable `CONFIG_LWIP_CHECK_THREAD_SAFETY` in debug builds so this
  class of bug asserts.

**D2. DNSResolver flags: a fast reply leaves the resolver stuck forever - Medium [verified]**

- Where: `DNSResolver.cpp:72-77` vs `:92-112`. After `ERR_INPROGRESS` the caller writes
  `_addrValid = false; _lookupInProgress = true;`. If the callback has already run, those stores
  overwrite its result and `getIPAddr` returns false forever, so the logger or MQTT client never
  connects until reboot.
- Fix: set `_lookupInProgress` before the call and undo it for other results; do the sequence under
  the tcpip lock; atomic flags.

**L1. `LoggerRaftRemote` ring buffer freed while other tasks write to it - High [verified]**

- Where: `RaftSysMods/LogManager/LoggerRaftRemote.cpp:65, 101` (producer, any task that logs) vs
  `:395-407` (`vRingbufferDelete(_ringBuf); _ringBuf = nullptr;` on main, on every remote-log
  client disconnect or socket error).
- Today the window is between the delete and the null store, and it lines up with network loss:
  `sys_evt` logs "WiFi station disconnected" at about the moment the TCP client fails. With main on
  core 1 any core-0 task that is inside `xRingbufferSend` races the free.
- Fix: never free the ring buffer once created; gate producers with an atomic "client connected"
  flag and flush on a new connection.

**L2. `LoggerCore::_loggers` vector mutated while other tasks iterate it - Medium, boot only.**
`RaftCore/core/Logger/LoggerCore.cpp:80-89` vs `:162-166`. `NetworkManager` starts before
`LogManager::setup()`, so `sys_evt` can be logging while `addLogger` reallocates the vector. Fix: a
fixed array with an atomic count (write the slot, then increment).

**Low items.**

| ID | Where | Issue |
|---|---|---|
| L3 | `LoggerLoki.cpp:130-140`, `LoggerPapertrail.cpp:71-85`, `LoggerRaftRemote.cpp:69-79` | rate-limit counters are unsynchronised read-modify-writes from many tasks (miscounts only) |
| L4 | `LoggerBase.h:42-89`, `LoggerRaftRemote.cpp:444-466` | `_level`, `_isPaused`, window settings are plain variables read on any task |
| L5 | `LoggerCore.cpp:49-50, 136-151`, `SerialConsole.cpp:171-199` | logging blocks the calling task for up to 100 ms per segment on USB-JTAG and uses over 2 kB of the caller's stack; this includes `sys_evt`. Multi-segment writes interleave between cores (garbled console lines, more often after the change) |
| L6 | `LoggerLoki.cpp:101-113` | destructor can delete the ring buffer under a live worker (latent) |
| O2 | `ESPOTAUpdate.cpp:218, 287, 548-558` | depth-1 queue with `xQueuePeek` means a cancel during `esp_ota_begin` is dropped after 1 ms; `esp_ota_abort` is never called; `isBusy()` sticks |

Note: `RaftSysMods/devdocs/LoggerLoki_Implementation.md:16-32` is stale. It says the `ESP_LOG`
hook feeds `LoggerCore` (it does not; the only `esp_log_set_vprintf` hook writes to the console),
that Loki network I/O is on the main task, and that `log()` is ISR-safe (`xRingbufferSend` is not).

### 6.5 RaftI2C

**I1. I2C hardware is driven from non-worker tasks; the bus has no lock - High [verified]**

- Where: `RaftI2C/BusI2C/BusI2C.h:307-310`, `BusIOExpanders/BusIOExpander.cpp:111-151`,
  `BusI2C/DeviceIdentMgr.cpp:701-730`, `BusI2C/BusI2C.cpp:572-649, 724-728`,
  `I2CCentral/RaftI2CCentral.cpp:273-565`.
- `virtualPinRead()` passes `_busReqAsyncFn`, which is bound to `BusI2C::i2cSendAsync`
  (`BusI2C.cpp:56`) and performs the transaction inline on the caller's task. So a virtual pin read
  from the main loop does slot-enable, `access()` and slot-disable concurrently with the worker.
- `DeviceIdentMgr::sendCmdToDevice` falls back to the same inline path when the enqueue fails
  (queue full, or the 2 ms queue lock wait expires).
- `busReqSync()` and `clearBusStuck()` rely on a pause flag that has no owner. This is acknowledged
  in `RaftCore/core/Bus/RaftBusDevicesIF.h:206-208`.
- Main can run today exactly when the worker is blocked mid-transaction on the access semaphore.
  A second `access()` overwrites `_readBufStartPtr` (which points at the first caller's stack
  buffer), `ensureI2CReady()` sees the peripheral busy and resets it mid-transaction, and the mux
  mask cache goes out of step with the hardware.
- Effect: stack corruption, wrong-slot writes, phantom offline devices.
- Fix: `virtualPinRead` should enqueue through `BusAccessor::addRequest`. Remove the inline
  fallback and return a busy code. Add an owner mutex around the whole slot-enable, transaction,
  slot-disable sequence, or assert the caller is the worker task.

**I2. `BusStatusMgr::loop()` loses status changes - High [verified]**

- Where: `RaftI2C/BusI2C/BusStatusMgr.cpp:101-201`.
- Main harvests changes under the mutex (`:137-158`), calls user callbacks unlocked (`:174`), then
  erases **all** `PENDING_DELETION` records (`:178-196`), then clears
  `_busElemStatusChangeDetected` **outside** the mutex (`:200`).
- A change the worker records during the callbacks is stranded until some other device changes
  state: a newly online device is not reported and gets no data-callback registration. A device the
  worker marks `PENDING_DELETION` during the callbacks is erased without ever being reported, so
  status listeners never detach. Several devices going offline together (a cable or slot unplug)
  is the normal trigger.
- Related: if the flag is set but there are no changes, it is never cleared, and main then takes
  the mutex on every loop forever.
- Fix: clear the flag inside the locked harvest, before the callbacks. Erase only the addresses
  reported in this pass.

**I4. Waits for request completion can never succeed from the main loop - Medium/High [verified]**

- Where: `RaftI2C/BusI2C/DeviceIdentMgr.cpp:683-718`, `RaftCore/core/DeviceManager/DeviceManager.cpp:1199-1254`,
  `RaftI2C/BusI2C/BusAccessor.cpp:64-89, 275-281`.
- Non-poll completion callbacks are delivered only by `BusAccessor::loop()` on the main loop
  (`handleResponse` puts them on `_responseQueue`). `sendCmdToDevice` waits 500 ms on a semaphore
  that only that callback gives. `apiDevManCmdRaw` spins up to 20 ms on `_cmdRawResultReady`.
- All API handlers run on the main loop (section 3.2). So each such call **blocks the main loop for
  the full timeout and then reports failure**, although the worker did send the command. The
  comment at `DeviceManager.cpp:1236-1238` says "this can run on the main loop task"; the one at
  `:2399` says the callback "runs on the bus worker task", which is wrong.
- This is a direct source of `loop()` stalls unrelated to WiFi. Please confirm on hardware: a
  `devman/cmdraw` with `numToRd > 0` should currently always return `readTimeout`.
- Fix: for requests with a waiter, invoke the completion from the worker in `handleResponse`. Use
  the per-request `shared_ptr` completion block (already in `sendCmdToDevice`) for cmdraw too and
  carry the read data in it, which also removes the shared `_cmdRawReadData` state. Better still,
  make these APIs asynchronous so the main loop never waits.

**I5. Data-change callbacks run on the worker, and unregister never reaches the bus - Medium.**
`DeviceManager.cpp:1672-1715`, `BusStatusMgr.cpp:737-771, 1194-1211`. `registerForDeviceData(...,
unregister=true)` only removes the pending record in `DeviceManager`. The callback and its context
pointer already installed in the `BusAddrRecord` stay armed and the worker keeps calling them: a
use-after-free once the subscriber is destroyed. With main on core 1 the callback also runs truly
parallel with the subscriber's own `loop()`. Fix: an unregister path down to `BusStatusMgr`; or
queue data-change events to the main loop; at minimum document the task context.

**I6. Bus-task hook and new-device handler registration - Medium.**
`RaftI2C/BusI2C/DeviceIdentMgr.h:146-170`. Registration writes the function then the context; the
worker reads them with no lock, so it can call `fn(nullptr)`. Deregistration cannot wait for an
in-flight call. Apps that rely on "main cannot run while my handler is mid-step" (true today for
`taskCore:0` except when the handler blocks on the bus) will break after the change. Fix: store
`{fn, ctx}` as one unit behind a lock; add a quiescence handshake on deregister; document the
context.

**I7. BusAccessor polling list - Medium/Low.** `BusAccessor.cpp:93-98, 175-224, 262-274`.
`pause()` iterates `_pollingVector` without `_pollingMutex` while `addToPollingList` can
reallocate it **[verified]**. `_pollingMutex` is held across the whole I2C transaction and the user
callback, so a main-loop `addRequest(poll)` blocks for a full transaction, and a callback that adds
a poll request deadlocks the worker. Fix: copy the request out under the lock, transact unlocked,
re-lock to update; take the lock in `pause()`.

**Low/Medium items.**

| ID | Where | Issue | Fix |
|---|---|---|---|
| I8 | `BusIOExpander.cpp:44-102` | `virtualPinsSet` returns `RAFT_OK` when its 10 ms lock take fails and nothing was done | return a busy code or wait forever |
| I9 | `BusPowerController.cpp:286-361, 463-485` | slot power state raced between `enableSlot` (main) and the worker; a slot can end up powered when off was requested | lock the slot records or post requests to the worker |
| I10 | `BusScanner.cpp:310-323, 533-541` | `requestScan` rewrites scanner state from another task (skipped or duplicated probe) | atomic request flag consumed by the worker |
| I11 | `BusI2C.cpp:697-705`, `BusAccessor.cpp:150-158` | pause and hiatus flags have no owner; `FW_UPDATE` and `SEND_IF_PAUSED` requests still transact while paused | see I1 |
| I12 | `BusMultiplexers.cpp:687-709`, `SlotController.cpp:80-164` | unsynchronised read-modify-write of `disabledSlotsMask` and slot mode | atomics |
| I13 | `RaftBusStats`, `DevicePollingMgr::_crcStats` | counters incremented from several tasks | atomics or accept |
| I14 | `BusI2C.cpp:96-108, 249-263` | destructor deletes the I2C central if the 1 s wait for the worker expires | handshake |
| I15 | `BusI2C/PollDataAggregator.h` | no destructor, so one FreeRTOS mutex leaks per device re-identification | add destructor |

Planned work in `RaftI2C/devdocs` that this affects: `i2c-adaptive-yield-plan.md` plans to write
`_loopYieldEveryMs` "only before task start or while paused, no additional synchronisation needed"
(needs atomics once main is on another core); `serial-slot-control-design.md` requires a slot to be
removed from scanning atomically before the analog switch flips, which the current unlocked
`disabledSlotsMask` write does not guarantee.

Checked and safe: `DeviceTypeRecords` (reserved, append-only vector, all access under its mutex,
`getDeviceInfo` returns a copy); the `BusStatusMgr` address table, poll info and aggregators (all
under the mutex, consistent lock order, callbacks made outside the lock); the request and response
queues; scan priority lists (immutable after setup); `sendCmdToDevice`'s `shared_ptr` completion
block. No lock-order inversion was found in RaftI2C or DeviceManager.

### 6.6 DeviceManager, FileSystem, LEDPixels

**DM1. DeviceManager maps and callback lists are unsynchronised - Low by default, Medium with BLE
central (B7).** `DeviceManager.cpp:1668-1724, 1794-1798, 2020-2125`, `:248`. The name/role
`unordered_map`s and `_requestedDeviceDataChangeCBList` have no lock. All current callers are on
the main loop except the BTHome path, which reaches `busElemStatusCB` from the NimBLE host task.
The header comments (`DeviceManager.h:82-100`) say callbacks "may occur on different threads" but
nothing makes registration thread-safe. Fix: take `_accessMutex` in these accessors and iterate a
snapshot, or fix B7 and assert main-task.

**DM2. `_accessMutex` 5 ms timeouts silently drop work - Low by default.** `DeviceManager.cpp:1810,
1835, 1866, 1899, 2314`. On timeout `callDeviceStatusChangeCBs` returns without calling any
listener, permanently losing the change. The list it protects is immutable after `setup()`, so the
lock mostly creates a failure mode. Contention needs an off-main caller (B7). Fix: wait forever, or
drop the lock for the immutable list.

**FS1. FileSystem - Low/Medium (grouped).** `RaftCore/core/FileSystem/FileSystem.cpp`.

- `reformat()` (`:121-210`) takes no `_fileSysMutex` and rewrites `_localFsType` and the cache
  name Strings, which `checkFileSystem`/`getDefaultFSRoot` read unlocked.
- `sdRequestUsedBytesUpdate` (`:1894-1897`) is a check-then-set; two callers can start two scans.
- `sdUpdateUsedBytes` holds `_fileSysMutex` across `f_getfree` (`:1852-1868`), which can take
  several seconds. Every other file-system call waits forever, including main-loop calls, so the
  main loop can stall for seconds (section 7).
- 64-bit sizes are read after unlock in `fileInfoGenImmediate` (`:1734-1737`): torn value possible.

**LED1. LED pattern and pixel buffer have no lock - latent.** `RaftCore/core/LEDPixels/LEDSegment.h:86-231`,
`ESP32RMTLedStrip.cpp:244-380`. `stopPattern` deletes `_pCurrentPattern` while `loop()` may be
inside it, and two `showPixels()` calls can both pass the `_txInProgress` check. Both the `led/*`
API and `loop()` run on the main loop today, so this is not reachable in the default build. It
becomes High if any off-main caller is added (see WS3). Fix: document "main loop only" and assert,
or marshal API commands through a queue applied in `LEDPixelsDevice::loop()`.

### 6.7 RaftWebServer

**WS1. The WebSocket send path has no synchronisation and runs on the caller's task - High,
conditional/latent.**

- Where: `RaftWebServer/RaftWebConnManager.cpp:266-290, 383-504`, `RaftWebResponderWS.cpp:411-502`,
  `RaftWebConnection.cpp:1101-1353, 1594-1626`. Entry: `RaftCore/comms/CommsChannels/CommsChannelManager.cpp:546-566`,
  which sends PUBLISH messages inline and carries the author's own TODO: "maybe on callback thread
  here so make sure this is ok".
- Shared state: `_pResponder` and `_pClientConn` (deleted in `clear()`), `_socketTxQueuedBuffer`
  (`std::vector` resize/erase), `_connectionSlots`, the socket fd.
- Every framework caller is on main today, so this is not currently a bug. Any application that
  publishes to a WS channel from its own task (the comment at `WebServer.cpp:371` mentions
  "high-rate camera frames") gets use-after-free, heap corruption or interleaved WS frames, today,
  and much more often after the change.
- Fix: in `CommsChannelManager`, never send inline unless on the owner task; hold the latest
  publish in a per-channel slot and send from `loop()`. Or have `RaftWebConnManager` record the
  owner task and enqueue frames that arrive from other tasks. At minimum add a task-identity assert
  and document "main loop only". Enabling `WEBSOCKET_SEND_USE_TX_QUEUE` alone does not fix it.

**WS2. The listener task is created unpinned and `taskCore` is silently ignored - Low [verified].**
`RaftWebConnManager.cpp:84-86` passes `pinToCore=false`; `RaftThreading.cpp:91-106` then uses
`xTaskCreate`. The listener can already run on core 1. Its only shared object is the hand-off
queue, which is correct. Fix: pin it, deliberately, to core 0 alongside lwIP.

**WS3. `USE_THREAD_FOR_CLIENT_CONN_SERVICING` must not be used as a way to get the web server off
the main loop.** If enabled, every REST handler would race its SysMod's `loop()`: `ProtocolExchange`
sessions, `StatePublisher` subscriptions, `LoggerRaftRemote` teardown, `ESPOTAUpdate` start,
`LEDPixels`, `NetworkSystem` configuration and DeviceManager maps all become High. The option is
also bit-rotted (it does not compile). Fix: delete it, or mark it as requiring a full redesign with
API dispatch marshalled to the main loop.

**Low items.** WS4: stale comments in `RaftWebConnection.cpp:290-294, 1554-1557` describe a web
server task that does not exist. WS5: `heap_caps_check_integrity_all(true)` runs unconditionally on
every connection reset (`RaftClientConnSockets.cpp:241`); it walks every heap with the heap lock
held and will stall core-0 allocators from core 1. Guard it with a debug define.

Checked and safe: the listener-to-main hand-off queue (results checked, single ownership); the OTA
block hand-off (deep copy before queueing); `_webConnections` is never resized after setup.

### 6.8 Other comms

**Q4. `_commsChannelVec` can reallocate while the NimBLE task indexes it - Low, boot window.**
`CommsChannelManager.cpp:162` vs `:299-306`. NimBLE starts in `BLEManager::setup()` before later
SysMods register channels. Fix: `reserve()` a fixed maximum, or start NimBLE in `postSetup`.

**Q5. `RingBufferPosn::count()` can transiently report "full" - Low, unused outside unit tests.**
`RaftCore/core/RingBuffer/RingBufferPosn.h:106-114`. Put and get are correct for strict SPSC.

## 7. Blocking calls on the main loop (not fixed by the affinity change)

The original symptom was `loop()` stalling. Pinning to core 1 only removes preemption by core-0
tasks. These stall the loop from inside, on either core:

| Where | Worst case | Note |
|---|---|---|
| `DeviceIdentMgr::sendCmdToDevice` from an API handler | 500 ms every call | finding I4, verified |
| `apiDevManCmdRaw` with `numToRd > 0` | 20 ms every call | finding I4 |
| `FileSystem` calls while `SDUsedScan` holds `_fileSysMutex` across `f_getfree` | seconds | finding FS1 |
| `ESPOTAUpdate::fileStreamDataBlock` `xQueueSend(..., 5000)` | 5 s | normally avoided by `apiReadyToReceiveData` |
| `NetworkSystem::configWifiSTA` retry loop with `vTaskDelay(20)` | 2 s | `NetworkSystem.cpp:650-665` |
| `esp_wifi_*` calls from `NetworkSystem::loop()` and API handlers (`esp_wifi_sta_get_ap_info` every 2 s, `esp_wifi_connect`, `esp_wifi_scan_start`, `pauseWiFi` stop/start) | depends on WiFi task | these wait on the WiFi driver *(IDF behaviour, not read)*; a driver kept busy by a bad antenna delays the caller on any core |
| `BLEGapServer::updateRSSICachedValue` (`ble_gap_conn_rssi`, blocking HCI) | ms | |
| `BusAccessor::addRequest(poll)` while the worker holds `_pollingMutex` across a transaction | tens of ms | finding I7 |
| `LoggerCore` console write, `usb_serial_jtag_write_bytes` 100 ms timeout per segment | 100-200 ms per log line with no host attached (not confirmed) | finding L5 |
| `heap_caps_check_integrity_all` on connection reset | ms to tens of ms with PSRAM | finding WS5 |
| Flash erase/write (NVS writes by WiFi and config saves, OTA, LittleFS) | tens of ms | disables the cache and stalls **both** cores |
| `RaftClientConnSockets::sendDataBuffer` retry | 10 ms | |

**Diagnosing the original 275 ms stall.** The scaffold already sets `slowSysModMs: 50`, so the log
shows `loop sysMod <name> SLOW took <n>ms`. If it always names `NetMan`, the cause is a blocking
call inside the loop and the affinity change will not help. If the named SysMod varies, the cause
is preemption and the change is the right fix.

## 8. Recommended solutions

### 8.1 Design rules to adopt

1. **Framework state is owned by the main loop. Other tasks hand off through a queue or an
   atomic.** The code already does this correctly in three places: `NetworkManager::loop()` for
   status callbacks, `_sntpSyncPendingNotify` for SNTP, and the comms inbound queue. Apply the same
   pattern to BLE connection state (B1), WiFi and IP events (N1), advertising restarts (B3) and BLE
   central callbacks (B7).
2. **No Arduino `String`, `std::vector`, `std::list` or `RaftJson` access from event handlers,
   NimBLE callbacks or worker tasks** unless under a lock taken on both sides.
3. **Write the data before the flag** when signalling across tasks (B4, N4), and make the flag
   `std::atomic` or use the existing `RaftAtomicBool`.
4. **Never ignore a lock or queue result.** Fix `ThreadSafeQueue` (Q2) so it is hard to: blocking
   defaults, `[[nodiscard]]`, a distinct failure result from `count()`.
5. **lwIP raw API only in tcpip context** (D1, N5).
6. **Assert ownership in debug builds.** Add a `Raft::assertMainTask()` helper and call it in the
   entry points that are main-only by convention: `CommsChannelManager::outboundHandleMsg`,
   `RaftWebConnManager::sendBufOnChannel`, `NetworkSystem` control operations, `DeviceManager`
   registration, `LEDPixels` mutation, `RaftJson::setSourceStr`. This turns latent findings (WS1,
   LED1, DM1) into immediate, debuggable failures.
7. **Document the task context of every user callback** (the table in section 3.3) in the public
   headers.

### 8.2 Phased plan

**Phase 0 - confirm the diagnosis.** Check the SLOW logs from the faulty-antenna project (section
7). On that hardware, trial `CONFIG_ESP_MAIN_TASK_AFFINITY_CPU1=y` by hand. Confirm finding I4 on
hardware.

**Phase 1 - required before CPU1 becomes the scaffold default.**

| Finding | Change |
|---|---|
| Q1, Q2 | single checked `get()` in `processInboundQueue`; `ThreadSafeQueue` defaults and result handling |
| Q3 | create codecs on the main loop only |
| B1, N2, N3 | BLE status change dispatched from `BLEGapServer::loop()`; pausing flag set before `stopWifi()` |
| B4, N4 | atomic connection handle; timestamp-before-flag ordering |
| I3 | allocate the I2C interrupt on the worker's core; make the timeout path safe cross-core |
| O1 | OTA status lock waits |
| B2 | only if any product uses BLE `taskEnable=1` |

**Phase 2 - already bugs today; fix regardless.** I1, I2, I4, D1, D2, L1, N1, N5, B5(d), C1 with
B3, L2, I5.

**Phase 3 - hardening.** The remaining Medium and Low items; the debug asserts and callback-context
documentation from 8.1; `CONFIG_LWIP_CHECK_THREAD_SAFETY=y` in debug builds; pin the web listener
(WS2); remove or fence `USE_THREAD_FOR_CLIENT_CONN_SERVICING` (WS3); reduce the blocking calls in
section 7, in particular move the `esp_wifi_*` calls out of the loop (cache RSSI from a small
core-0 worker or from events, and turn `configWifiSTA` into a state machine).

**Phase 4 - RaftCLI scaffold change** (section 9).

### 8.3 Verification

- Build the unit tests and a test app with main pinned to core 1 and `CONFIG_FREERTOS_HZ=1000`.
- Stress cases that target the Phase 1 findings: BLE file upload and OTA while publishing at a
  high rate (Q1, B2, B4); repeated BLE connect/disconnect with `pauseWiFiforBLE=1` while polling
  `/api/sysmodinfo/NetMan` (B1, N1, N2); I2C with forced NACK timeouts and the worker on the other
  core from the ISR (I3); remote log client connect/disconnect in a loop under heavy logging (L1).
- Enable `CONFIG_LWIP_CHECK_THREAD_SAFETY`, heap poisoning (`CONFIG_HEAP_POISONING_COMPREHENSIVE`)
  and the task-identity asserts during these runs.
- The Linux build of RaftCore can run the queue and `RaftJson` tests under ThreadSanitizer.

## 9. The RaftCLI scaffold change (not applied)

Once Phase 1 is released, generate the setting for dual-core targets only. The option is invalid
on single-core chips (`esp32c3`, `esp32c5`, `esp32c6`), so it must be conditional on `target_chip`
being `esp32`, `esp32s3` or `esp32p4`.

- `raft_templates/systypes/{{sys_type_name}}/sdkconfig.defaults`: add a placeholder next to
  `CONFIG_ESP_MAIN_TASK_STACK_SIZE`, for example `{{{main_task_affinity_sdkconfig}}}`.
- `src/app_config.rs`: add a generator entry following the `flash_size_*_sdkconfig` pattern. The
  condition evaluator appears to support only `==`, so this needs either one entry per dual-core
  chip or a small extension to allow a set match.
- Generated text:

  ```
  # Run the main task (Raft loop) on core 1, away from WiFi/BT on core 0
  CONFIG_ESP_MAIN_TASK_AFFINITY_CPU1=y
  ```

- Prefer `CPU1` over `NO_AFFINITY`: on ESP32 and S3 an unpinned task is pinned to whichever core it
  is on at its first FPU use, so the result is not predictable.
- Leave the Raft worker tasks (`BusI2C`, OTA, web listener, BLE) on core 0 by default. Note that an
  I2C bus configured with `"taskCore": 1` would then share core 1 with the main loop and, at
  priority 5, outrank it.
- The 1 ms `delay()` at the end of `SysManager::loop()` lets the core-1 idle task run, so the task
  watchdog is unaffected provided `CONFIG_FREERTOS_HZ=1000` (the scaffold sets it). At lower tick
  rates `delay(1)` becomes `vTaskDelay(0)` and never yields to idle.
- Existing projects are unaffected by a template change. They need the line added by hand and
  their `sdkconfig` regenerated. The release notes should say that the Phase 1 library versions
  are a prerequisite.
- Also consider changing the scaffold default `"pauseWiFiforBLE": 1` until B1 is fixed, since it is
  what makes B1 active in every scaffolded BLE app.

## 10. Non-concurrency bugs noticed in passing

- `CommsChannel.cpp:35`: the inbound queue's maximum **count** is built from `inboundBlockLen`
  (default 1200) rather than `inboundQueueCountMax` (default 20) **[verified]**.
  `inboundQueueBytesMax` is never enforced. If the main loop stalls, a BLE producer can queue ~1200
  messages and exhaust the heap; back-pressure is effectively off.
- `SysManager.cpp:594-597`: `clearAllStatusChangeCBs` returns inside the loop, so only the first
  SysMod is cleared.
- `RaftJsonNVS.cpp:78-97`: error paths return without `nvs_close`.
- `BLEGattOutbound.cpp:282`: `txMsg(len, rslt)` receives the enum `..._OK == 0` as a bool, so every
  successful send is counted as an error.
- `BLEGattOutbound.cpp:119-121, 353-355`: a command arriving while a publish message is part-sent
  is interleaved into it, corrupting HDLC framing.
- `BLEGattServer.cpp:523-529`: `handleSubscription` sets `_responseNotifyState` from any
  attribute's subscribe event and only from `cur_notify`.
- `BLEStdServices.h:84-92`: the access callback always uses `_standardServices.back()`.
- `WiFiScanner.cpp:34-38`: `_scanInProgress` stays true if `esp_wifi_scan_start` fails.
- `NetworkSystem.cpp:276`: `esp_netif_sntp_init` is called again every 10 hours without a deinit.
- `BusI2C.h:318-323`: `enableSlot` dereferences `_pBusPowerController` with no null check;
  `devman/slot` crashes on a bus with no `"pwr"` config.
- `RaftDevice.cpp:251-252`: `registerForDeviceData` dereferences a null bus for direct-connected
  devices.
- `BusStatusMgr`: offline and pending-deletion records are still returned by
  `getPendingIdentPoll`, so they keep being polled.
- `FileSystem.cpp:2071-2102`: `fileSystemCacheService` marks file info valid after fixing only the
  first invalid entry.
- `PollDataAggregator`: `uint16_t` offsets overflow when `numSamples * resultSize > 65535`.
- `ESPOTAUpdate`: cancel never calls `esp_ota_abort` (finding O2).
- `RaftMQTTClient.cpp:259-272, 323-325`: the closed-connection path never closes the fd; publish
  ignores send errors.
- `CommandSerial.cpp:64`: `setup` configures `_serialPorts.back()` on every iteration.
- `WebServer.cpp:125-128`: every config change adds another static-files handler; `:238, 254`
  index `_certsTempStorage[size()-1]` when the size can be 0.
- `RaftWebResponderSSEvents.cpp` is not built and would not compile; the SSE API is a stub.

## 11. Open questions

- Which products use BLE `taskEnable=1` (B2), BLE central/BTHome (B7, DM1), `CommandSocket`
  (a second off-main producer for Q1 and Q3), or `"taskCore": 1` for I2C (I3 today).
- Whether application SysMods publish, call `outboundHandleMsg`, `virtualPinRead`, `busReqSync`,
  `pause()`, `requestScan()` or `registerBusTaskServiceHandler()` from their own tasks (WS1, I1,
  I6, I10).
- Project values of `CONFIG_LWIP_TCPIP_CORE_LOCKING`, `CONFIG_BT_NIMBLE_PINNED_TO_CORE`, the NimBLE
  host stack size and the `sys_evt` stack size.
- NimBLE behaviour not read from source: whether `nimble_port_stop` always delivers disconnect
  events first; what happens when a second indication is sent with one outstanding; whether the S3
  controller ever allocates connection handle 0.
- The behaviour of `usb_serial_jtag_write_bytes` with no host attached in the IDF version in use
  (decides whether L5 stalls for the full timeout on every log line).
- The C5/C6 `RaftI2CCentral_ESPIDF` variant was not reviewed in depth. Those are single-core chips,
  so the affinity change does not apply to them.
