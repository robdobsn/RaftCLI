# `raft new`: an "app type" question that tunes the defaults

Date: 2026-10-27. Status: **phase 1 implemented** (RaftCLI PR: `app_type` with `general` and `battery`, §5.1-5.5, §6
tests; §7's hardware measurements still to do). Phases 2-4 are plans. Written after the PulseCounterESP32C5 work (a
battery-powered pulse counter), where the settings in §3 were found one at a time and could have been set by
`raft new` in one go. Reviewed 2026-10-27 against RaftCore, RaftSysMods, PulseCounterESP32C5 and the `raft new`
code; the corrections are in §8.

Read with `raft-new-questions-and-defaults-review.md` (the question set and what each choice costs) - this builds
on that machinery rather than replacing it.

---

## 1. The idea

`raft new` asks about hardware and features (chip, connectivity, web server, I2C...). It doesn't ask what *kind*
of app this is, yet that decides a bundle of settings across every layer of a project:

| Layer | Examples |
|-|-|
| `SysTypes.json` (runtime config) | NetMan `startMode`, `wifiSTAFastReconnect`, `wifiBand`, `wifiAPEn`, `enableMDNS`; BLEMan `connIntvPrefMs`, `pubQSize`, `pubQMaxBytes`; WebServer `sendMax` |
| `features.cmake` | `RAFT_SDKCONFIG_FRAGMENTS`, `RAFT_DEVICE_MANAGER`, `RAFT_NO_NETWORKING` |
| `sdkconfig.defaults` | log levels, BT on/off, main task core, CPU frequency, PSRAM and running from it, lwIP limits |
| `main.cpp` / user SysMod | which SysMods are registered; later, a skeleton suited to the type |

Plan: one `choice` question - **App type** - asked after the hardware and before the connectivity question. Its
answer sets the *defaults* of the later questions, and the type **together with the connectivity answer and the
chip** decides the generated settings (§4). Every later question still works as now, so a type is a starting
point, not a lock-in. `--set app_type=battery` and `--defaults` work as for any other question, and the answer is
saved in `.raftnew.json` like the others.

The types:

| Type | Meaning | Status |
|-|-|-|
| `general` | Today's app - the default; nothing changes for existing users | Phase 1 |
| `battery` | Sleeps most of the time, wakes to measure and report | Phase 1 (the worked example, §3) |
| `publishing` | Streams data (sensors at hundreds of Hz, camera frames) to a connected client | Phase 2 |
| `gateway` | Mains powered, reads sensors (I2C, BTHome over BLE) and forwards them (MQTT, web) | Phase 3 |

"Minimal / serial only" is not a type: it is the connectivity answer `none`, which already sets
`RAFT_NO_NETWORKING`, no web server and (since RaftCLI PR #9) one app partition.

---

## 2. Why now

Two things make this cheap to build:

- **RaftCore sdkconfig fragments** (RaftCore PR #37, merged 2026-10-27): `set(RAFT_SDKCONFIG_FRAGMENTS
  wifi-fast-connect deep-sleep-fast-wake)` in `features.cmake` layers named ESP-IDF settings between
  `systypes/Common/sdkconfig.defaults` and the SysType's own. A type can select fragments by name instead of
  `raft new` writing CONFIG lines it then has to keep in step with RaftCore. The two fragments so far:
  `wifi-fast-connect` (`CONFIG_LWIP_DHCP_RESTORE_LAST_IP`, `CONFIG_LWIP_DHCP_DOES_NOT_CHECK_OFFERED_IP`) and
  `deep-sleep-fast-wake` (`CONFIG_BOOTLOADER_SKIP_VALIDATE_IN_DEEP_SLEEP`, `CONFIG_BOOTLOADER_LOG_LEVEL_WARN`). See
  RaftCore `sdkconfig-fragments/README.md`.
- **NetMan on-demand start and fast reconnect** (RaftCore #37, RaftSysMods #15, both merged 2026-10-27): the
  battery-device behaviour is NetMan config (`startMode`, `wifiSTAFastReconnect`, `wifiSTAFastReconnectMaxUses`,
  `wifiBand`, `enableMDNS` - all read in RaftCore `NetworkSettings.h`) rather than app code. See RaftCore
  `devdocs/wifi-fast-connect-and-on-demand.md`.

---

## 3. The battery type, worked out

What a battery device wants, with where each setting goes and why. The "in PulseCounter" column says whether the
real SysType (`systypes/PulseCtr`) uses it, or only the power-measurement SysType (`PulseCtrPower`), or neither.

| Setting | Layer | Why | In PulseCounter |
|-|-|-|-|
| NetMan `"startMode": "onDemand"` | SysTypes | WiFi only on the wakes that report; the app sends `{"cmd":"wifiStart"}` to NetMan when it needs it | yes |
| NetMan `"wifiSTAFastReconnect": 1`, `"wifiSTAFastReconnectMaxUses": 30` | SysTypes | Connect straight to the remembered AP, rescanning now and then for a better one | yes |
| NetMan `"wifiBand": "2g"` (chips with 5 GHz: esp32c5) | SysTypes | A 2.4 GHz-only scan is far shorter (no passive DFS channels); 5 GHz transmit draws about 400 mA against about 310 mA | yes |
| NetMan `"wifiAPEn": 0`, `"enableMDNS": 0` | SysTypes | No access point or mDNS: nothing to answer, shorter sessions. The `w/SSID/password` command still provisions over serial | yes |
| `RAFT_SDKCONFIG_FRAGMENTS wifi-fast-connect` | features.cmake | DHCP restores the last lease without an ARP check (2-3 s saved when not using a static IP) | yes |
| `RAFT_SDKCONFIG_FRAGMENTS deep-sleep-fast-wake` | features.cmake | No app image check on a deep-sleep wake; bootloader warnings only | **no** - only suggested in a comment in `PulseCtrPower`; measure before making it a default (§7) |
| ESP-IDF log level WARN (`CONFIG_LOG_DEFAULT_LEVEL_WARN`) | sdkconfig | ESP-IDF logging lengthens every wake (Raft's `LOG_I` isn't affected by this level) | `PulseCtrPower` only; `PulseCtr` logs at INFO |
| `CONFIG_BT_ENABLED=n` unless BLE chosen | sdkconfig | The connectivity question already does this | yes |
| `RAFT_DEVICE_MANAGER OFF` unless I2C chosen | features.cmake | About 100 KB of flash | yes (PulseCounter has no I2C) |
| OTA: keep | partitions | Updates over WiFi on a maintenance wake are the only way to update an installed device | yes |
| Web server: default off | SysTypes / features | Nothing is awake long enough to browse to | yes |
| Static IP | per device, not SysTypes | `wifiip/<ip>/<gateway>` API - an installation's address, so not in the generated config; the generated README says how to set it | yes (NVS) |

Things the type must **not** decide, because they're the app's: the sleep/wake schedule, what to upload, the
server protocol, and per-device values (static IP, broker address, credentials). The generated README gets a
section naming the commands that set the per-device values (`w`, `wifiip`, the MQTT settings) so they stay out of
SysTypes.

**Measured** (RaftCore `devdocs/wifi-fast-connect-and-on-demand.md`, ESP32-C5, ESP-IDF 6.1, deep-sleep wake to IP
address): defaults 15.3 s; 2.4 GHz only with a scan 0.85-3.1 s; remembered AP with a static IP 0.26-0.31 s. The
fast figure is with the first five rows of the table **and a static IP**, not with `deep-sleep-fast-wake` or WARN
logging - so the type's defaults are expected to give about 0.3 s plus 2-3 s of DHCP until the installer sets a
static IP, and §7 measures the two unproven rows separately.

---

## 4. Settings by type and connectivity

A type alone doesn't decide the settings: battery + BLE wants none of the WiFi settings, and publishing over BLE
wants different things from publishing over WiFi. So the derivation is a set of rules on (type, connectivity,
chip), applied in the derive step (§5.3). Only `general` keeps every setting as it is today.

### 4.1 Defaults of later questions

| Question (`--set` key) | `general` | `battery` | `publishing` | `gateway` |
|-|-|-|-|-|
| Wireless connectivity (`connectivity`) | `wifi_ble` | `wifi` | `wifi_ble` | `wifi_ble` |
| BLE role (`ble_roles`) | `peripheral` | `peripheral` | `peripheral` | `both` (BTHome scanning) |
| Web server (`use_raft_webserver`) | `true` | `false` | `true` | `true` |
| OTA (`ota`) | `true` | `true` | `true` | `true` |
| I2C and sensors (`i2c_sensors`) | `common` | `none` | `common` | `common` |
| Decoding (`runtime_decode`) | `attrs` | `attrs` | `attrs` | `code` |

A `--set` or saved answer always wins over a type default.

### 4.2 Generated settings

| Rule | Settings |
|-|-|
| battery, WiFi | NetMan block of §3 (`startMode onDemand`, fast reconnect, no AP, no mDNS); fragment `wifi-fast-connect`; `wifiBand 2g` only when the chip has 5 GHz (esp32c5) |
| battery, BLE | BLEMan `advIntervalMs` long (e.g. 1000 - advertising is most of a sleeping device's radio time); no WiFi settings |
| battery, any | fragment `deep-sleep-fast-wake` and WARN log levels **once §7 has measured them**; MQTTMan block present but `enable 0`, as today |
| publishing, BLE | BLEMan `connIntvPrefMs 7.5`, `pubQSize 64`, `pubQMaxBytes 65536`; `CONFIG_BT_NIMBLE_PINNED_TO_CORE_0` (the main task is on core 1) - from the 2026-10-25 camera-over-BLE work (RaftSysMods PR #12) |
| publishing, WiFi | WebServer `sendMax 250000` (default 20000; the Axiom plant board's value for camera frames) |
| publishing, esp32s3 with 8 MB octal PSRAM | `CONFIG_SPIRAM_XIP_FROM_PSRAM=y` (run the app from PSRAM, as Axiom018: flash writes no longer stall other tasks). Not with 2 MB of PSRAM, which the app would mostly fill |
| gateway | MQTTMan `enable 1`; BLEMan `scanBTHome 1` when the role includes central; DevMan as today |

Already the default for every type, so not type settings: main task on core 1 (`main_task_core`), 240 MHz on the
esp32/esp32s3, lwIP 24 sockets, `RAFT_LOGGER_USB_JTAG_WRITE_TIMEOUT_MS=10`. Measured 2026-10-25 and found not
to help, so no type sets them: lwIP core locking, a larger TCP send buffer (`CONFIG_LWIP_TCP_SND_BUF_DEFAULT`).

### 4.3 Things a type doesn't change

- StatePublisher rates: clients choose the rate when they subscribe; there is nothing to set on the device.
- The partition layout beyond the OTA answer (RaftCLI `new_layout.rs` sizes it from the flash).
- Anything per device (§3).

---

## 5. Implementation

All in RaftCLI unless stated. File references are to `src/app_config.rs` unless stated.

### 5.1 The question

Add to the question list (after the `hardware` question, before `connectivity`):

```json
{ "key": "app_type", "prompt": "App type", "default": "general", "datatype": "choice",
  "choices": [
    { "value": "general",    "label": "General - connected device with a web interface (the usual defaults)" },
    { "value": "battery",    "label": "Battery powered - sleeps, wakes to measure and report over WiFi" },
    { "value": "publishing", "label": "High-rate publishing - streams sensor data or camera frames to a client" },
    { "value": "gateway",    "label": "Sensor hub / gateway - reads sensors and forwards them (MQTT, BTHome)" }
  ] }
```

Phase 1 ships `general` and `battery` only; the other two labels are added with their phases, so the question
never offers a type whose settings aren't implemented.

### 5.2 Type defaults for later questions

Question defaults are fixed strings today (`add_default_value_to_context` sets them before any question is asked;
the asked branch renders `question.default` as a Handlebars template). Neither can depend on `app_type`, which
isn't known at that point. Smallest change:

- A table in Rust, `fn type_defaults(app_type: &str) -> &[(&str, &str)]`, with the §4.1 columns.
- Immediately after `app_type` is answered (asked, or from `overrides`), for each `(key, value)` in the table
  **where `key` is not in `overrides`**: `set_value(key, typed_value(question, value), ...)`. This replaces the
  fixed default in `responses`, so a skipped question (`--defaults`) gets the type's default.
- In the asked branch, use the current `responses` value as the prompt's default when a type default exists for
  the key (today it re-renders `question.default`). One `if` before `render_template`.

Not a map inside the JSON question list: the question list is already long, and the table is one place to read
all of a type's defaults.

### 5.3 Generated settings (the derive step)

`derive_values()` already computes `wifi`, `ble`, `webserver`, `i2c` and writes template variables with
`set_value`. Add the §4.2 rules there, writing new variables:

| Variable | Replaces / goes in | Content |
|-|-|-|
| `netman_systypes` | the `inc_netman_in_systypes` generator (keep its MQTTMan half as `mqttman_systypes`) | The NetMan block: today's for `general`, §3's for battery + WiFi (`wifiBand` added for esp32c5) |
| `bleman_systypes` | the `inc_bleman_in_systypes` generator | Today's block with `advIntervalMs` / `connIntvPrefMs` / `pubQSize` / `pubQMaxBytes` from the rules |
| `webserver_systypes` | the `inc_webserver_in_systypes` generator | Today's block, plus `"sendMax": 250000` for publishing + WiFi |
| `sdkconfig_fragments_cmake` | new, in `systypes/Common/features.cmake` after `{{{networking_cmake}}}` | `set(RAFT_SDKCONFIG_FRAGMENTS ...)` with a comment naming the type, or empty |
| `type_sdkconfig` | new, in the SysType's `sdkconfig.defaults` after `{{{psram_sdkconfig}}}` | Log levels (battery), `CONFIG_BT_NIMBLE_PINNED_TO_CORE_0` and `CONFIG_SPIRAM_XIP_FROM_PSRAM` (publishing), each with a comment saying why and that it's safe to remove |
| `device_manager_cmake` | new, in `systypes/Common/features.cmake` | `set(RAFT_DEVICE_MANAGER OFF)` when `i2c_sensors` is `none` - for every type (the review doc's §4.4 saving), not only battery |

Moving the three `inc_*_in_systypes` generators into Rust also removes JSON-in-a-string from the question list,
which is where mistakes have been hard to see.

### 5.4 Summary and saved answers

- `print_summary()` shows the type and the fragments selected.
- Nothing to add for saving: every answered question goes into `.raftnew.json` already, and `raft update-project
  --set app_type=battery` re-renders through the same `get_new_app_config()`. One thing to document: because the
  saved answers include every question, changing `app_type` later changes only the §4.2 settings, not answers
  already given (those are "overrides"). The README says to pass the question too (`--set use_raft_webserver=false`).
- Not `build/raft.info`: that is per-machine state (last port, baud), not project state.

### 5.5 The generated README

`raft_templates/README.md` gets a short section per type where there is something to say. For battery: the
on-demand WiFi (`{"cmd":"wifiStart"}` to NetMan from the app, or `wifistart` over the API), the per-device
commands (`w/<SSID>/<password>`, `wifiip/<ip>/<gateway>`, MQTT settings), and a pointer to RaftCore's
`wifi-fast-connect-and-on-demand.md`.

### 5.6 The user SysMod skeleton (not in the first version)

A wake/report/sleep skeleton (decide on wake whether this wake reports, with the decision in RTC memory; start
WiFi through NetMan; wait for MQTTMan's `connected` named value; do the work; arm the wake source; deep sleep) would
help a battery app most. But it is a lot of template code to keep current, and the general part - the wake
decision and the upload state machine - belongs in a library: a RaftSysMods base SysMod that an app's SysMod
derives from, with PulseCounterESP32C5's `UploadSchedule` and `PulseLogger` as the reference. Build that once a
second battery app shows what is actually general; until then the battery type sets configuration, and the
generated user SysMod is today's.

---

## 6. Tests

`cargo test` (in `app_new.rs`, with the `generate()` helper used by `partitions_flash_and_psram`):

- `app_type_defaults`: `--set app_type=battery` alone gives `connectivity wifi`, no web server, no I2C in the
  generated files; `--set app_type=battery --set use_raft_webserver=true` keeps the web server (the override wins).
- `battery_wifi_settings`: the NetMan block has `startMode onDemand`, `wifiSTAFastReconnect`, `wifiAPEn 0`,
  `enableMDNS 0`; `features.cmake` has `RAFT_SDKCONFIG_FRAGMENTS wifi-fast-connect`; with `target_chip esp32c5`
  the block has `wifiBand 2g`, with esp32s3 it doesn't.
- `battery_ble_settings`: `--set app_type=battery --set connectivity=ble`: no NetMan block, no fragments line, a
  long `advIntervalMs`.
- `general_unchanged`: the files generated for `general` (the default) are byte-identical to those before the
  change, for the three connectivity cases the review doc's §6 builds. (Generate them from the previous release
  into a fixture before starting, so the test can be written first.)
- Phase 2: `publishing_ble_settings`, `publishing_wifi_settings`, `publishing_xip_only_with_8mb_octal_psram`.
- `update_project_app_type`: `raft update-project --set app_type=battery` on a `general` app rewrites the NetMan
  block and `features.cmake` and leaves edited files beside `.raftnew` copies.

All tests run with `--locked` in CI (RaftCLI PR #10).

---

## 7. Validation on hardware

The review doc's method: generate each type with `--defaults`, build with `raft build -i`, record app size and
free slot space in a table in this document. Then, for battery, on an ESP32-C5 board:

1. Flash the generated battery app with the §3 rows PulseCounter uses. Measure wake-to-IP (`tIp` in the
   PulseCounter state message, or NetMan's named values) over 20 wakes with DHCP, and again with a static IP. Expect
   about 0.3 s with a static IP and 2-3 s more with DHCP.
2. Add `deep-sleep-fast-wake` and measure again. If it saves a measurable part of the wake, it becomes a battery
   default; otherwise it stays a documented option.
3. Set the ESP-IDF log level to WARN and measure again, likewise.
4. Check that `w/<SSID>/<password>` over serial still provisions a device with the AP and mDNS off.

For publishing (phase 2): the plant growth board's camera over BLE and WebSocket, with the 2026-10-25 figures as
the baseline (BLE: frames intact, longest loop about 5 ms; WebSocket SVGA publish 10.5 ms).

---

## 8. Review notes (2026-10-27)

Corrections to the first draft, kept here so the reasoning isn't lost:

- "All of these are in use in PulseCounterESP32C5" was true of the NetMan rows and `wifi-fast-connect` only.
  `deep-sleep-fast-wake` is not selected anywhere in that project, and WARN logging is only in the measurement
  SysType. The 0.26-0.35 s figure was measured with a static IP. §3 and §7 now say so.
- Recording the type in `raft.info` was the wrong place (per-machine state, and tracked by mistake). The answer is
  saved in `.raftnew.json` with no extra work.
- Type-dependent defaults need the code change in §5.2; a map in the JSON `default` would not work as the code
  stands, because a skipped question's default is never rendered.
- The "high-rate publishing" sketch listed settings that are already the defaults (core 1, 240 MHz, lwIP 24) and
  one that isn't a device setting (StatePublisher rates). §4.2 replaces it with the measured BLE and WebSocket
  settings from the camera work.
- "Minimal / serial only" is the connectivity answer `none`, not a type.
- The two-axes question (power source × workload): one question with four types is enough, because the settings
  are derived from (type, connectivity, chip) rather than from the type alone.
- Remove questions or change defaults? Change defaults, except where a question makes no sense for the other
  answers - the pattern RaftCLI PR #9 set for OTA without wireless. Battery + WiFi could skip the web server
  question the same way, but a long-lived battery device with a web page exists (a display on mains-assisted
  power), so it only changes the default.

## 9. Phases

| Phase | Work | Done when |
|-|-|-|
| 1 | §5.1-5.5 with `general` and `battery`; §6 tests; §7 steps 1-4; README (RaftCLI and the generated app); wiki RaftCLI page | A battery app from `raft new --set app_type=battery` wakes to an IP in about 0.3 s with a static IP on the C5, and `general` output is unchanged. **Done 2026-10-27 except §7 steps 1-4** (no C5 board attached): `general` output byte-identical for all four connectivity cases; a C5 battery app builds (1.29 MB app, fragment applied) |
| 2 | `publishing`: §4.2 rules, tests, measured on the plant growth board | Camera over BLE and WebSocket match the 2026-10-25 figures without hand-editing SysTypes |
| 3 | `gateway`: MQTTMan on, BTHome scanning, tests | An app reads an I2C sensor and a BTHome sensor and publishes both over MQTT |
| 4 (RaftSysMods) | Battery base SysMod (§5.6), then a `battery` user SysMod template that derives from it | PulseCounterESP32C5's upload schedule moves onto it |

## References

- RaftCore `devdocs/wifi-fast-connect-and-on-demand.md` - the NetMan settings and measurements
- RaftCore `sdkconfig-fragments/README.md` - fragment names and layering
- RaftSysMods PR #12 (BLE publish queue limits, `pubQMaxBytes`), RaftWebServer PRs #10 and #11 (WebSocket sends)
- PulseCounterESP32C5: `systypes/Common/features.cmake`, `systypes/PulseCtr/SysTypes.json`,
  `systypes/PulseCtrPower/sdkconfig.defaults`, `components/PulseLogger/UploadSchedule.*`
- Here: `raft-new-questions-and-defaults-review.md` (question set, size measurements, `--set`/`--defaults`),
  `main-task-core1-concurrency-audit.md` (why the main task is on core 1)
