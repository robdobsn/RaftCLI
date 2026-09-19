# Raft concurrency hardening: changes made

Date: 2026-09-18
Companion to `main-task-core1-concurrency-audit.md`. Finding IDs (Q1, B1, I3 ...) refer to that document.

## 0. Update 2026-09-19

- The library changes described below have been merged to `main` in RaftCore, RaftSysMods, RaftI2C and
  RaftWebServer (they are not yet in tagged releases). Section 1 describes the state when they were made.
- Decision (revised later on 2026-09-19): the main task core is a `raft new` question, asked only for chips
  with more than one core (esp32, esp32s3, esp32p4), defaulting to core 1. Answering 1 generates
  `CONFIG_ESP_MAIN_TASK_AFFINITY_CPU1=y` in the SysType's `sdkconfig.defaults`; answering 0, or a single-core
  chip, generates nothing (`main_task_core*` entries in `src/app_config.rs`, covered by unit tests). This
  replaces audit section 9.
- Hardware test of a scaffolded app with the main task on core 1 (Xiao ESP32-S3, IDF 6.0.2, local library
  working trees): boot log shows `main_task: Started on CPU1`, no errors or thread-safety warnings; idle loop
  0.27 ms average / about 1.2 ms maximum (0.38 ms / 3-5 ms on core 0); `blerestart` re-advertises; a 60 s
  REST API soak over WiFi gave 2131 requests with 0 failures and a loop maximum of 23 ms. BLE connections,
  I2C devices and OTA were not exercised (no central or devices attached).
- Two pre-existing library bugs found during that testing were fixed (uncommitted at the time of writing):
  RaftSysMods `BLEGapServer.cpp` `sysmodinfo/BLEMan` returned invalid JSON (missing quote after `advName`);
  RaftCore `NetworkSystem::getConnStateJSON` reported `conn:0` when WiFi had never been paused (it now only
  uses the before-pause value while paused, and no longer emits `RSSI`/`IP` twice when paused).
- RaftCLI scaffold changes that were made as a result of the analysis:
  - Console: `CONFIG_ESP_CONSOLE_USB_SERIAL_JTAG` is now only generated for chips that have the peripheral;
    `esp32` gets `CONFIG_ESP_CONSOLE_UART_DEFAULT` (`console_*_sdkconfig` generators in `src/app_config.rs`).
  - `systypes/Common/features.cmake` sets `RAFT_LOGGER_USB_JTAG_WRITE_TIMEOUT_MS=10` (finding L5: with no USB
    host reading, each console log write could block the calling task for the RaftCore default of 100 ms).
    The RaftCore default itself is unchanged.
    Measured 2026-09-19 on a Xiao ESP32-S3 (IDF 6.0.2, scaffolded app, USB connected but the port not
    open): with both 10 ms and the 100 ms default the main loop stayed at about 0.38 ms average and under
    5 ms maximum, and the 10 s report interval did not drift (10001 ms per report) while log lines were
    being dropped. So the 100-200 ms per log line stall suspected in finding L5 does **not** occur in that
    situation; the setting is harmless but made no measurable difference. Not tested: board powered with
    no USB host at all, and other IDF versions.
  - `features.cmake` has a commented `RAFT_MAIN_TASK_CHECK_ABORT` definition, and `sdkconfig.defaults` a
    commented debug block (assertions, `CONFIG_LWIP_CHECK_THREAD_SAFETY`, heap poisoning). The lwIP check is
    an assertion, so it does nothing unless `CONFIG_COMPILER_OPTIMIZATION_ASSERTIONS_DISABLE` is also removed.
  - The generated SysMod and README document which task runs what, and that the libraries must be kept in step.
  - Unit tests in `src/app_config.rs` cover the console generators and check the text templates render.

## 1. Status

- Changes are in the working trees of four repos on a new local branch `concurrency-hardening`
  (RaftCore, RaftSysMods, RaftI2C, RaftWebServer). **Nothing is committed or pushed.** RaftMotorControl
  and RaftCLI (scaffold) are unchanged.
- **Nothing has been run on hardware.** Verification so far is compile-only:
  - every source file of all five libraries passes `-fsyntax-only` with the real toolchains, zero errors
    and zero warnings, using the compile commands of existing builds repointed at the working trees:
    ESP32 with NimBLE + Ethernet + SoftAP (ScaderRelays, 137 files), ESP32-S3 (RaftCore `unit_tests`,
    67 files; RaftI2C was also checked against an S3 build with `-Wall -Werror`);
  - RaftCore `linux_unit_tests` builds and passes.
  - Not compiled: ESP32-C3/C5/C6 targets, RaftI2C `unit_tests`. A full `raft build` of RaftCore
    `unit_tests` was attempted but ninja failed with "manifest still dirty" (a file timestamp problem,
    not a compile error).
- The four repos must be released together: RaftSysMods, RaftI2C and RaftWebServer now depend on new
  RaftCore items (`RaftMainTask.h`, `ThreadSafeQueue::pop()`, `RaftBusDevicesIF::unregisterForDeviceData`).
- The scaffold change (audit section 9) has **not** been made. It should wait until these library
  changes have been tested on hardware and released.

## 2. New building blocks in RaftCore

| Item | Where | Purpose |
|---|---|---|
| `RAFT_CHECK_MAIN_TASK(prefix, fnName)` | `core/Utils/RaftMainTask.h` | Placed at main-task-only entry points. Logs a throttled `LOG_E` if called from another task. Define `RAFT_MAIN_TASK_CHECK_ABORT` to `abort()` instead (recommended for debug builds) or `RAFT_MAIN_TASK_CHECK_DISABLE` to remove. |
| `RaftThread_setMainTask()` / `RaftThread_isMainTask()` | `core/Utils/RaftThreading.*` | `SysManager::loop()` records its task as the main task. `isMainTask()` returns true until a main task has been recorded, so `setup()` code never trips the check. |
| `RaftAtomicBool_exchange()` | `core/Utils/RaftThreading.*` | Atomic test-and-set. |
| `RaftMutex_lock` hardening | `core/Utils/RaftThreading.cpp` | Returns false for a mutex that failed to create; a non-zero timeout is now always at least 1 tick (previously a 5 ms wait at a 100 Hz tick silently became a try-lock). |
| `ThreadSafeQueue` rework (Q2) | `core/ThreadSafeQueue/ThreadSafeQueue.h` | Default lock wait 10 ms instead of 0; `put/get/peek/pop` are `[[nodiscard]]`; `count()` and `canAcceptData()` are lock-free and cannot fail; new `pop()`; `clear()` returns bool; non-copyable; `_maxLen` is 32-bit. |

`std::atomic` is now used in these libraries (it was not before). Classes that gained atomic members are
no longer copyable; all existing uses were checked.

Entry points that now have the main-task check: `CommsChannelManager::outboundHandleMsg`, the
`NetworkSystem` control operations (`pauseWiFi`, `configWifiSTA`, `configWifiAP`, `clearCredentials`,
`wifiScan`, `setHostname`), `LEDSegment::setPattern/stopPattern`, `BLEGapServer::startAdvertising`,
and `RaftWebConnManager::sendBufOnChannel`, `canSendBufOnChannel`, `isChannelConnected`,
`serverSideEventsSendMsg`. An application that publishes or sends from its own task will now see the
error log; it was already unsafe (WS1).

## 3. Findings by status

### Phase 1 (blockers for the core-1 change)

| ID | Status | What changed |
|---|---|---|
| Q1 | Done | `CommsChannel::processInboundQueue` checks codec readiness, then does one checked `get()`; a message is only processed if it was actually removed. |
| Q2 | Done | See section 2. All callers in the four repos handle results. |
| Q3 | Done | Codecs are created only from `CommsChannelManager::loop()`. `inboundHandleMsg`, `inboundHandleMsgVec`, `inboundCanAccept` and the block-max getters no longer create them (they are called from the NimBLE host task). The "no codec" warning is throttled to once per 10 s. |
| Q4 | Done | `_commsChannelVec` reserves 20 entries in the constructor. |
| B1, N3 | Done | `BLEGapServer::setConnState` only stores state atomically. `BLEGapServer::loop()` detects the edge and calls the status-change hooks on the main task, so `pauseWiFi` no longer runs on the NimBLE host task. Disconnect-then-connect between two loops reports both; connect-then-disconnect between two loops reports nothing. |
| N2 | Done | `_isPaused` is atomic and set **before** `stopWifi()` (and cleared before `startWifi()` on resume). |
| B4 | Done | One `std::atomic<uint16_t>` connection handle with `BLE_HS_CONN_HANDLE_NONE` as the sentinel, in both `BLEGapServer` and `BLEGattServer`, snapshotted once per function. Timestamp-before-flag for the conn-interval check, restart state and advertising check. |
| N4 | Done | `requestMDNSSetup()` writes the time, then the atomic flag. |
| I3 | Done | The I2C interrupt is allocated on the worker's core (`initOnBusTask()` called at worker start, with a lazy fallback in `access()`). Access start and end are inside the `_i2cAccessMutex` critical section for every outcome; the whole ISR body runs inside the same section and does nothing unless `_accessInProgress` is set. A zero-timeout drain of `_accessSemaphore` precedes each access. |
| O1 | Done | OTA status mutex waits 100 ms for statistics and forever where the result is recorded; the OTA is never failed because of the lock; CRC computed outside it. |
| B2 | Done | A message counts as removed only if `pop()` succeeded; puts use a 10 ms wait. |

### Phase 2 (already bugs today)

| ID | Status | What changed |
|---|---|---|
| I1 | Done | Recursive bus-owner mutex in `BusI2C`. The worker holds it for each pass (never across its yield delay); `i2cSendSync`, `i2cSendAsync`, `busReqSync`, `clearBusStuck` take it with a 100 ms limit and return `RAFT_BUSY` on timeout. The inline fallback in `sendCmdToDevice` is removed. Lock order is documented in `BusI2C.h`. |
| I2 | Done | The change flag is cleared inside the locked harvest, before callbacks; only records reported as pending-deletion in this pass (and still so on re-lock) are erased. |
| I4 | Done | `sendCmdToDevice` no longer waits when called on the main task (returns `RAFT_OK`, "Command queued"); other tasks still wait up to 500 ms and that wait can now succeed. `apiDevManCmdRaw` services the bus (`pBus->loop()`) while it waits when on the main task, so the read result can arrive; the callback then runs on the same task. |
| D1 | Done | `dns_gethostbyname` runs in tcpip context via `esp_netif_tcpip_exec`. `getIPAddr` must not be called from the tcpip thread (documented; all current callers are `loop()` functions or the Loki worker). |
| D2 | Done | The in-progress/valid flags are set inside the same tcpip-context call, before the lookup, so a fast reply can no longer be overwritten. Flags are atomic; the address is written before the flag. |
| L1 | Done | The remote-log ring buffer is never freed once created; producers are gated by an atomic "client connected" flag; stale contents are flushed on a new connection. |
| N1 | Done | `_wifiStaSSID`, `_wifiIPV4Addr`, `_ethIPV4Addr`, `_ethMACAddress` are only accessed through `getConnInfoStr()/setConnInfoStr()` under a mutex taken on both sides; readers work from copies. Event handlers log from locals. |
| N5 | Done | `netbiosns_init/set_name` run in tcpip context. The `esp_netif_next_unsafe` walk is replaced by `esp_netif_get_handle_from_ifkey("ETH_DEF")`. `setHostname` only sets the NetBIOS name once NetBIOS has been started. |
| B5 | Done (a: partial) | (d) GATT service tables are built once, so restart no longer hands NimBLE a dangling pointer. (c) `nimbleStop()` result checked before re-init. (b) off-host NimBLE calls gated by an atomic host-ready flag. (a) the outbound task is asked to exit and given 500 ms before the stack stops; falls back to `vTaskDelete` after that. |
| C1, B3, B14 | Done | Host-task events only set an atomic "advertising start required" flag; `startAdvertising()` runs only from `BLEGapServer::loop()`, so config is no longer read on the host task. `RaftJson.h` now documents that config is main-task-only. |
| L2 | Done | `LoggerCore` holds loggers in a fixed array (max 8) with an atomic count; slot written before the count. `clearLoggers()` no longer frees anything another task might be using. |
| I5 | Done | New `RaftBusDevicesIF::unregisterForDeviceData(address, info)` and `unregisterForDeviceDataAll(info)`, implemented by RaftI2C (disarms under the mutex and waits up to 500 ms for an in-flight callback) and by the BLE bus. `DeviceManager::registerForDeviceData(..., unregister=true)` calls them. |

### Phase 3 items also done

N6, N7 (atomics; scan flag reset on failure and on pause; it was also uninitialised), B6, B8 (counters),
B9, B10, B11, B12, B13, B7 (BTHome status/data callbacks now raised from the main task), L4, L6, O2
(`esp_ota_abort` on cancel and failure; cancel can no longer be lost, `isBusy()` cannot stick), C2 (MAC
string cache removed; null provider is a function-local static), DM1 (callback lists locked, iterated
from a snapshot), DM2 (`_accessMutex` waits forever; it is never held across a callback), FS1
(`reformat` takes `_fileSysMutex`; `f_getfree` runs without it; scan start is a test-and-set), LED1
(check + docs), I6, I7, I8, I9, I10, I11, I12, I14, I15, WS1 (checks + docs), WS2 (listener pinned to
`taskCore`, falling back to core 0 if invalid), WS3 (dead code removed), WS4, WS5 (heap check behind
`DEBUG_HEAP_ON_LIFECYCLE`). Callback task contexts are documented at the typedefs in
`RaftDeviceConsts.h`.

Section 10 bugs fixed along the way: `clearAllStatusChangeCBs` returning inside its loop;
`RaftJsonNVS::setJsonDoc` not closing NVS on error paths; BLE `txMsg` counting every success as an
error; command messages interleaved into a part-sent publish message; `BusI2C::enableSlot` null
dereference with no `"pwr"` config; WebServer adding a static-files handler on every config change;
`_certsTempStorage[size()-1]` with size 0; `esp_intr_alloc_intrstatus` result never assigned.

## 4. Behaviour changes to be aware of

- BLE advertising starts one main-loop iteration after sync/disconnect instead of inside the host event.
- BLE status-change callbacks (and therefore `pauseWiFiforBLE`) now happen up to one loop later.
- BTHome: several adverts from one device between two loops are coalesced to the latest.
- `LoggerRaftRemote`: the 16 kB ring buffer stays allocated after the first client; `bufsize` only
  applies before the first connection.
- `sendCmdToDevice` from the main task returns before the result is known.
- `virtualPinRead`/`busReqSync` from a non-worker task can block up to 100 ms for the bus-owner lock and
  then return `RAFT_BUSY`; `virtualPinRead` now returns real failures instead of `RAFT_OK`.
- User poll and device-data callbacks on the I2C worker now run with the bus-owner lock held.
- The I2C ISR now holds its spinlock for its whole body (including the Tx FIFO fill).
- The web listener task now honours `taskCore` (it was silently unpinned).
- WebServer `staticFilePaths` changes need a restart (as port and slot count already do).
- BLE `txB` statistics now count bytes actually sent.
- OTA `fileStreamCancelEnd` returns true when the queue is full (the cancel flag still guarantees delivery).
- `LoggerCore` supports at most 8 loggers.

## 5. Not done, and why

| Item | Reason |
|---|---|
| `CommsChannel.cpp:35` inbound queue length uses `inboundBlockLen` (section 10) | Every channel uses the default count of 20, so the fix would cut BLE's effective inbound depth from ~500-1200 messages to 20 and could break BLE upload on write-without-response. Left with a TODO; needs a hardware test and probably a larger default. |
| `apiFirmwareMain` can report "InProgress" as failure if called before the worker finishes `esp_ota_end` | Logic race separate from the lock; needs a design decision (wait for the worker, or report pending). |
| WS1 send path made thread-safe | Out of scope by design; enforced as main-task-only instead. |
| General `shared_ptr` document swap in `RaftJson` (C1 general fix) | Off-main readers were removed instead. |
| L3 logger rate-limit counters, I13 bus statistics, `MovingRate` index race (B8) | Miscounts only. |
| B6 sequence number for stale indication ACKs | Counter is now atomic and clamped; the sequence number was not added. |
| L5 console logging blocking the caller; section 7 blocking calls (`esp_wifi_*` from `loop()`, `configWifiSTA` retry loop, OTA `xQueueSend(5000)`) | Phase 3 redesign work, not concurrency fixes. `apiDevManCmdRaw` still blocks the main loop for up to 20 ms by design. |
| `LoggerLoki` destructor wait is unbounded | Latent; the logger is never destroyed at runtime. |
| `serial-slot-control-design.md` atomic slot removal | The mask is now atomic, but the worker can still be mid-transaction on the slot when the switch flips. |
| `CONFIG_LWIP_CHECK_THREAD_SAFETY` in debug builds; scaffold `CONFIG_ESP_MAIN_TASK_AFFINITY_CPU1` | sdkconfig/scaffold changes; do after hardware testing. |
| Remaining section 10 items (`handleSubscription`, `BLEStdServices` `.back()`, SNTP re-init, MQTT fd leak, `CommandSerial` `.back()`, FileSystem cache service, `PollDataAggregator` overflow, `RaftDevice` null bus) | Not concurrency related; left for separate fixes. |

## 6. Hardware verification needed

Use main pinned to core 1, `CONFIG_FREERTOS_HZ=1000`, `RAFT_MAIN_TASK_CHECK_ABORT`,
`CONFIG_LWIP_CHECK_THREAD_SAFETY=y` and `CONFIG_HEAP_POISONING_COMPREHENSIVE=y`.

1. BLE: advertising after boot, after disconnect and after two consecutive `blerestart`s (B5d);
   connect/disconnect cycling with `pauseWiFiforBLE=1` while polling `sysmodinfo/NetMan` (B1, N1, N2);
   file upload and OTA over BLE while publishing (Q1, B2, B4), also with `taskEnable=1`.
2. I2C: forced NACKs and timeouts with the worker on the opposite core to main; confirm the ISR lands
   on the worker's core and there are no false timeouts after a real one; 400 kHz FIFO reads at
   208/416 Hz; poll throughput with the owner lock; multi-device unplug (I2); `devman/cmdraw` with
   `numToRd > 0` (should now return data) and `devman/slot` with and without `"pwr"`.
3. Network: WiFi connect/disconnect cycling; mDNS and NetBIOS discovery; hostname change; Papertrail,
   Loki and MQTT name resolution after a network drop (D1, D2); Ethernet builds.
4. Logging: remote-log client connect/disconnect in a loop under heavy logging (L1).
5. OTA: success, cancel during `esp_ota_begin`, cancel followed by a new OTA.
6. SD card: file listing on the local file system while the SD used-bytes scan runs (FS1).
7. Check the logs for "called from task other than main" in every application; each one is a real
   thread-safety problem in the caller.
