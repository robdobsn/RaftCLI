# `raft new`: questions and defaults - review and proposal

Date: 2026-10-25. Status: **implemented 2026-10-25** (§10), with the proposals of §9 as decided. Not yet merged;
not yet built or run on boards beyond the checks in §10.

`raft new` asks up to 18 questions. With the default answers it makes an app that **only just fits its app
partition**. This document measures what each choice costs and proposes a shorter, hardware-aware set of
questions. That set must still work:
- with no board to query;
- for a module or a bare chip rather than a dev board;
- for a user who doesn't know their hardware.

---

## 1. The default app today

`raft new newapp --defaults` with RaftCLI 1.30.0:
- ESP32-S3, 4 MB flash, ESP-IDF 6.1;
- a user SysMod, Raft SysMods and the web server;
- BLE peripheral, I2C on pins 5/6, and all of RaftCore's device types (59 records).

It builds cleanly with `raft build -i`. `newapp.bin` is **1,672,976 bytes in a 1,769,472-byte app slot: 5.5% free**
(about 94 KB). The 4 MB layout has two OTA slots of 0x1B0000, so a user adding real code will soon overflow it.

---

## 2. What each choice costs (measured)

**Method:**
- The default app above and eight variants were built, each generated with `raft new --defaults` plus `--set`
  overrides or a one-line edit, then built with `raft build -i`.
- Target: ESP32-S3, ESP-IDF 6.1.0, size-optimised (`CONFIG_COMPILER_OPTIMIZATION_SIZE`).
- Libraries: RaftCore `d3dd3f3`, RaftSysMods 1.27.0, RaftWebServer 1.18.0, RaftI2C `8b9021c` (all `main`,
  2026-10-25).
- Static RAM figures are from `esp_idf_size` on each map file.

| Variant | App bytes | Change | Free in a 1.69 MB slot | Static internal RAM (DIRAM) |
|-|-|-|-|-|
| Default (all of the above) | 1,672,976 | — | 5.5% | 123,455 |
| No BLE | 1,430,368 | **−242,608** | 19.2% | 99,275 (−24 KB) |
| No web server | 1,634,544 | −38,432 | 7.6% | 123,423 |
| No device types (none compiled) | 1,591,808 | −81,168 | 10.0% | 123,455 |
| 10 common device types¹ | 1,611,712 | −61,264 | 8.9% | 123,455 |
| Runtime decoding off (`RAFT_RUNTIME_DECODE OFF`) | 1,655,232 | −17,744 | 6.5% | 123,199 |
| Runtime decoding + code interpreter (`RAFT_RUNTIME_DECODE_CODE ON`) | 1,699,104 | +26,128 | 4.0% | 123,739 |
| No I2C (so no device types) | 1,498,496 | −174,480 | 15.3% | 122,435 |
| No BLE, no web server, no device types | 1,310,176 | −362,800 | 26.0% | 99,243 |
| **No WiFi** (`RAFT_NO_NETWORKING`, §10; no BLE, web server or device types) | 581,296 | **−728,880** from the row above | 67.1% | — |
| **BLE only** (as above, with BLE) | 870,400 | BLE costs 289 KB without WiFi | 50.8% | — |

¹ SHT40, SHT31, BME280, BME680, SCD40, VL53L1X, LSM6DS3, MCP9808, VEML7700, MAX17048. They cost 19.9 KB, about 2 KB each.
All 59 records cost 81 KB, about 1.4 KB each. Popular records are larger than average (custom decode code).

² *Before §10's change, no WiFi couldn't be built, and this was estimated.*
- In ESP-IDF 6.1 `CONFIG_ESP_WIFI_ENABLED` has no prompt and is always on for a WiFi chip.
- RaftCore lists `esp_wifi` as a requirement.
- RaftSysMods already leaves networking out when `CONFIG_ESP_WIFI_ENABLED` is off, but it never is.

The estimate is the WiFi- and IP-only libraries in the minimal build, which come to 434 KB of flash:
- `net80211` 144 KB, `lwip` 78, `wpa_supplicant` 66, `pp` 48, `mdns` 31, `phy` 29;
- `esp_netif`, `esp_http_client`, `esp-tls`, `tcp_transport`, `http_parser` and `esp_wifi`, 39 together.

Not counted: part of the 140 KB crypto libraries (`tfpsacrypto`, `mbedtls`, `mbedx509`), and these libraries'
string literals. The size tool attributes all merged strings (about 180 KB) to one object (`stdio_vfs.c`).

**Runtime RAM.** BLE and WiFi also allocate tens of KB of heap at runtime, which the static figures don't show.
Measure on a board if a "connectivity" choice is to quote RAM.

---

## 3. The questions today

| # | Question (prompt) | Default | Shown when |
|-|-|-|-|
| 1 | Project Name | folder name | always |
| 2 | System Type Name | project name | always |
| 3 | Target Chip | esp32s3 | always |
| 4 | CPU core for the main task | 1 | esp32, esp32s3, esp32p4 |
| — | Use SPIRAM (PSRAM) | false | **commented out since 2024-11**: PSRAM boards get no `CONFIG_SPIRAM` |
| 5 | Flash Size in MB | 4 | always |
| 6 | ESP-IDF Version | 6.1 (RaftCLI's default) | always |
| 7 | Create User SysMod | true | always |
| 8 | User SysMod Class | MainSysMod | user SysMod |
| 9 | User SysMod Name | the class | user SysMod |
| 10 | Use Raft SysMods | true | always |
| 11 | Use Raft Web Server | true | always |
| 12 | Add support for Raft BLE | true | always |
| 13 | BLE Peripheral | true | BLE |
| 14 | BLE Central (for BTHome) | false | BLE |
| 15 | Add support for I2C | true | always |
| 16 | I2C SDA pin | 5 | I2C |
| 17 | I2C SCL pin | 6 | I2C |
| 18 | Include Raft Core Device Types | true | I2C |

**Not asked, but fixed in the templates:**
- The partition layout per flash size. It's always two OTA slots: 1.69 MB at 4 MB, 2 MB at 8 MB and larger.
- An open WiFi AP, `RaftAP`, with the password `raftpassword`.
- All 59 device types when device types are on.
- Runtime decoding on (the RaftCore default), and the code interpreter off.

---

## 4. Proposals

### 4.1 Hardware: chip, flash, PSRAM and pins from one question

One question ("What hardware?") with four routes. Each ends with the same five facts:
- chip;
- flash size;
- PSRAM (size and mode);
- I2C pins (if I2C);
- console (USB Serial/JTAG or UART).

**1. Detect.** Offered when a serial port with an Espressif device is found.
- `esptool flash-id` (RaftCLI already uses esptool for `raft flash`) reports:
  - the chip and its revision;
  - its features, including **embedded** PSRAM (e.g. "Embedded PSRAM 2MB (AP_3v3)");
  - the detected flash size.
- The results are shown for confirmation.
- **Limits:**
  - external PSRAM (e.g. ESP32-WROVER) isn't reported, so PSRAM is asked if the chip can have it and none was
    detected;
  - pins can't be detected, so they come from a known board or are asked;
  - under WSL the port is on Windows, so detection uses the Windows route `raft flash` already has
    (`raft.exe`).

**2. A dev board** from a list, e.g. ESP32-S3-DevKitC-1 (N8R8 and others), Unexpected Maker FeatherS3/TinyS3,
Seeed XIAO ESP32-S3/C3/C6, M5Stack AtomS3, Adafruit QT Py ESP32-S3.
- A preset gives all five facts. The FeatherS3, for example, has I2C on 8/9, not the template default 5/6.
- Presets live in a data file in RaftCLI (`board_presets.json`): chip, flash, PSRAM, PSRAM mode, I2C pins, console,
  and notes. Anyone can add one by PR.
- Unlisted boards use route 3 or 4.

**3. A module**, by Espressif's ordering code.
- The code says flash and PSRAM directly: `N<flash MB>[R<PSRAM MB>]`, e.g. ESP32-S3-WROOM-1-**N8R2**.
- RaftCLI asks for the module family, then the code (or accepts the full name). It parses `N`/`R`, and knows from
  the family:
  - the chip;
  - the PSRAM mode: on the ESP32-S3, R2 is quad and R8 is octal, which needs `CONFIG_SPIRAM_MODE_OCT`;
  - any pins the module uses itself: octal PSRAM takes GPIO 33-37 on the S3, so they mustn't be used for I2C.
- Families to start with:
  - ESP32-S3-WROOM-1/-1U: N4, N8, N16, N4R2, N8R2, N16R2, N4R8, N8R8, N16R8;
  - ESP32-S3-MINI-1: N8, N4R2;
  - ESP32-WROOM-32E: N4, N8, N16;
  - ESP32-WROVER-E: R8 variants, external PSRAM;
  - ESP32-C3-MINI-1 and ESP32-C3-WROOM-02: N4;
  - ESP32-C6-WROOM-1: N4, N8, N16;
  - ESP32-C6-MINI-1: N4.

  The exact list and pin reservations are to be checked against Espressif's datasheets before implementing (N4).
- Pins are asked (no board to say), with the chip's usual defaults, and the module's reserved pins are refused.

**4. A chip, or "I don't know".**
- Asks for the chip, then flash size and PSRAM, with **safe defaults**: 4 MB and no PSRAM. Those run on almost any
  board of that chip, so a wrong guess is an unused resource, not a broken build.
- "I don't know" for the chip defaults to esp32s3, as now.
- Raft says how to find out: plug the board in and run `raft new` again, or `raft detect` (a new command that runs
  route 1 on its own).
- It also says the choice can be changed later (§4.6).

**When no board is available (case a):** routes 2-4 need no board. Route 1 is only offered when a port is found, and
otherwise isn't mentioned.

**Modules and bare chips (case b):** routes 3 and 4.

**Unknown hardware (case c):** route 4 with safe defaults, the hint about detecting, and a way to change it later.

### 4.2 Connectivity: none / WiFi / BLE / WiFi + BLE

- **Replaces:** "Use Raft SysMods", "Use Raft Web Server" and "Add support for Raft BLE" (questions 10-12).
- **When BLE is chosen,** its role is asked: peripheral (for apps and the raft CLI), central (for BTHome), or both.
  This replaces questions 13-14. A user who picks "WiFi + BLE" without thinking about roles gets peripheral, as
  now.
- **When WiFi is chosen,** "Include the web server" is asked (yes by default), as question 11 now.
- **"None" and "BLE"** need RaftCore and RaftSysMods to build without networking (§7).
- **Cost:** BLE 237 KB and 24 KB of static RAM; WiFi about 450-550 KB.

### 4.3 OTA updates

- Yes (two app slots, as now) or no (one large slot).
- **Default:** yes, unless the slot would be under about 1.9 MB (4 MB flash). Then the summary shows the choice and
  its effect.
- **Without OTA:** the 4 MB slot grows from 1.69 MB to about 3 MB. Updates are then only over USB, and
  `ESPOTAUpdate` isn't registered (it saves about 6 KB).

### 4.4 Device types and decoding

**I2C and sensors:** no I2C / I2C with no device types / common / extended / all. One question, replacing today's 15
and 18. No I2C saves 93 KB beyond the device types (174 KB with them). The device type sets cost about 0 / 20 / 40 /
81 KB.
- The sets are defined in RaftCore, not RaftCLI: e.g. `devtypes/sets/common.json`, selected in
  `DeviceTypeSources.json` as `"raftcore:@common"`.
- A list inside RaftCLI would go stale when records are added or renamed, and an unknown key fails the build.
- More can always be added at runtime with a device type file.

**Decoding of devices added at runtime** (from a type file): none / attributes / attributes + code.
- These cost −17 / 0 / +26 KB from today's default.
- Compiled-in records always have compiled decoders, so this only affects devices added later.
- **Default:** attributes, as now.
- Ask only when device types are chosen.

### 4.5 Questions that become defaults or advanced

- **System Type Name:** the project name.
- **User SysMod Name:** the class name.
- **Main task core:** 1 on dual-core chips.
- **"Use Raft SysMods":** derived from connectivity.

These and the ESP-IDF version are shown in the summary, with `--advanced` to ask them.

### 4.6 Changing it later, and saving the answers

- **Save the answers:** `.raftnew.json` in the project, with the RaftCLI version.
- **Use them:**
  - `raft new --answers <file>` reproduces a project;
  - a bug report can include the answers;
  - `raft update-project --set flash_size=8` (or a new `raft reconfigure`) re-renders the hardware-dependent files:
    `partitions.csv`, the flash and PSRAM `sdkconfig` lines, and the I2C pins in `SysTypes.json`. It shows a diff
    and keeps the user's own edits.
- This is what makes the safe defaults of route 4 cheap: they can be corrected without starting again.

### 4.7 A summary before generating

- **Shows:** each choice, the partition layout, and an estimated app size and free space (from §2's table, kept with
  the CLI).
- **Warns** below 10% free, and suggests no OTA, fewer device types or no BLE.
- **After the first build** the real size is known. `raft build` already prints it; it could repeat the warning.

### 4.8 The default WiFi access point

Every new app starts an AP, `RaftAP`, with the published password `raftpassword`. Either:
- generate a random password at `raft new` (written into SysTypes.json and printed with the next steps); or
- leave the AP off by default and turn it on with a question.

Random by default is the smaller change (N9).

### 4.9 Partition layout computed, not chosen

App sizes aren't a question; few users could answer it. The layout comes from the flash size and OTA, as below:

| Flash | OTA | App slot(s) | File system |
|-|-|-|-|
| 4 MB | yes | 2 × 1.69 MB (as now) | 512 KB |
| 4 MB | no | 1 × 3 MB | about 900 KB |
| 8 MB | yes | 2 × 3 MB (now 2 MB) | about 1.9 MB |
| 16 MB | yes | 2 × 3 MB | about 9.9 MB |
| 32 MB | yes | 2 × 3 MB | about 25.9 MB |

The rule: the largest app slots up to 3 MB that leave at least 512 KB of file system. This is to be checked against
OTA and file system needs (N7).

---

## 5. The questions after the change

What is asked depends on the answers. The gain is **better questions and correct hardware**, not a much shorter
list. Counting each prompt:

| Route | Prompts |
|-|-|
| Known dev board | 11: project, hardware route, board, connectivity, BLE roles, web server, OTA, I2C and sensors, decoding, user SysMod, class name |
| Detected board | 12: as above, with the I2C pins asked instead of the board (two prompts) |
| Chip or "don't know", every branch | 15 |
| Today | 18 |

Ways to shorten it further:
- ask BLE roles only with `--advanced` (default peripheral);
- ask the SysMod class only with `--advanced` (default `MainSysMod`).

That gives 9 for a known board.

| # | Question | Covers today's | Shown when |
|-|-|-|-|
| 1 | Project name | 1 (and 2, 9 derived) | always |
| 2 | Hardware: detected / dev board / module / chip or don't know | 3, 5, the PSRAM question; 16-17 for presets | always |
| 2a | Board, module family + ordering code, or chip | — | by route |
| 2b | Flash size, PSRAM | 5, PSRAM | route 4 (and PSRAM if external PSRAM is possible) |
| 2c | I2C pins | 16-17 | I2C and no preset |
| 3 | Connectivity: WiFi + BLE / WiFi / BLE / none | 10-12 | always |
| 3a | BLE roles: peripheral / central / both | 13-14 | BLE |
| 3b | Web server | 11 | WiFi |
| 4 | OTA updates | — (new) | always |
| 5 | I2C and sensors: no I2C / I2C, no device types / common / extended / all | 15, 18 | always |
| 6 | Decode devices added at runtime: none / attributes / + code | — (new) | device types chosen |
| 7 | Create a user SysMod, and its class name | 7-8 | always (class only if yes) |
| — | Summary: estimated size, layout, ESP-IDF version; confirm or change | 4, 6, and advanced | always |

**Where today's questions go.** Each of the 18 is still covered:
- **Still asked (8):** 1, 3, 5, 7, 8, 13, 14, 15. Some are combined: 3 and 5 under hardware, 13 and 14 as BLE roles,
  15 with 18.
- **Derived (4):** 2 from 1, 9 from 8, 10 from connectivity, 12 from connectivity.
- **Moved or merged (6):**
  - 4 and 6 to the summary, or `--advanced`;
  - 11 to connectivity (3b);
  - 16-17 to presets, or asked;
  - 18 into the I2C and sensors question.

**New:** hardware route, OTA, runtime decoding, and the summary.

---

## 6. The three cases

| Case | What happens |
|-|-|
| **(a) No board to query** | Detection isn't offered. The user picks a dev board, a module or a chip; nothing needs a connection. `raft detect` can fill in the facts later, before `raft update-project` |
| **(b) A module or a bare chip** | Route 3: the family and ordering code (ESP32-S3-WROOM-1-N8R2 gives ESP32-S3, 8 MB flash, 2 MB quad PSRAM, with octal PSRAM pins refused for R8). Pins are asked. A bare chip with its own flash: route 4, asked for flash and PSRAM |
| **(c) Doesn't know the board** | Route 4 with safe defaults (4 MB, no PSRAM, chip esp32s3 unless chosen), a hint to plug in and detect, and the summary saying what was assumed. A wrong guess wastes resources but builds and runs, and can be changed later (§4.6) |

---

## 7. Work outside RaftCLI

- **RaftCore:** named device type sets (`devtypes/sets/*.json`, `"raftcore:@<set>"` in `DeviceTypeSources.json`,
  in the assembler, raftjs's selection rules and the shared vectors).
- **RaftCore and RaftSysMods:** a build without networking. A Raft-level define (e.g. `RAFT_NETWORKING 0`) that
  leaves out `NetworkSystem`, WiFi diagnostics and the networking SysMods, and drops `esp_wifi` from the
  requirements. Then measure the real saving.
- **Docs:** the RaftCLI README's "Scaffolding Questions", and the wiki's QuickStart.

---

## 8. Phases

| Phase | Work | Verify |
|-|-|-|
| 1 | Re-enable PSRAM (asked); save answers (`.raftnew.json`, `--answers`); random AP password; the summary with size estimates; System Type and SysMod names derived | Generate and build each chip's default |
| 2 | Hardware routes: board presets, module ordering codes, chip with safe defaults; `raft detect` (esptool, including the WSL route) | Unit tests of the ordering-code parser and presets; detection on an S3 dev board, a FeatherS3, and a WROOM module board |
| 3 | OTA question and computed layouts | Build and flash each layout; OTA update on 8 MB |
| 4 | RaftCore device type sets, and the sensors question | Builds with each set; sizes as §2 |
| 5 | Networking-free build in RaftCore/RaftSysMods; connectivity question | Build none / BLE / WiFi / both; measure flash and runtime heap |
| 6 | `raft update-project` re-rendering hardware files from saved answers | Change flash size and pins on a generated project; diff is as expected |

---

## 9. Decisions

| # | Question | Proposal |
|-|-|-|
| N1 | Hardware question with four routes (detect, board, module, chip/don't know) | Yes |
| N2 | Where board presets live | A data file in RaftCLI, extended by PR |
| N3 | Detection command | `raft detect`, also offered inside `raft new` when a port is found |
| N4 | Module families supported first | ESP32-S3-WROOM-1/-1U, -MINI-1, ESP32-WROOM-32E, ESP32-WROVER-E, ESP32-C3-MINI-1/-WROOM-02, ESP32-C6-WROOM-1/-MINI-1, checked against datasheets |
| N5 | Connectivity question, and the networking-free build it needs | Yes; RaftCore/RaftSysMods work first |
| N6 | Device type sets: names and contents | common (about 10), extended (about 25), all; defined in RaftCore |
| N7 | Layout rule (§4.9) and the OTA default on 4 MB | As §4.9; OTA yes by default, with the summary showing the slot size |
| N8 | Runtime decoding question | Yes, only when device types are chosen; default attributes |
| N9 | Default AP | Random password generated at `raft new` |
| N10 | Saving answers and `raft update-project` re-rendering | Yes, `.raftnew.json` |

---

## 10. Implementation (2026-10-25)

**RaftCore** (`feature/raft-new-support`):
- **Named sets:** `devtypes/sets/common.json` (10 records) and `extended.json` (25). They are selected as
  `"<bundled source>:@<set>"` in `include` or `exclude`, and published in the Pages `catalogue.json`. Sets are only
  for the bundled source; an unknown set, or a set naming a missing record, stops the build. Tests and shared
  selection vectors cover them.
- **No networking:** `RAFT_NO_NETWORKING` (a project-wide define) removes `SysManager`'s uses of `networkSystem`,
  and with them the "reboot if disconnected" check.

**RaftSysMods** (`feature/no-networking`): with `RAFT_NO_NETWORKING`, the networking SysMods (NetMan, CommandSocket,
MQTTMan) aren't registered, and LogManager's network loggers aren't created.

**raftjs** (`feature/devtype-sets`): the TypeScript selection rules handle sets (shared vectors), and
`RaftCoreCatalogue` passes them on.

**Measured without networking:** RaftCore and RaftSysMods were built from the branches, with no BLE, web server or
device types.
- The app is 581,296 bytes, against 1,310,176 with WiFi.
- The WiFi, lwIP, wpa_supplicant and mDNS libraries are gone from the map.
- BLE only is 870,400 bytes.
- **Not run on a board yet.**

**RaftCLI** (`feature/raft-new-questions`):

`src/new_hardware.rs`:
- the four hardware routes, the board and module data (`src/new_data/*.json`, entries marked "verify"), ordering
  codes, and esptool `flash-id` parsing for v4 and v5;
- `raft detect`. Under WSL it uses Windows `raft.exe detect`, or, with an older `raft.exe`, the Windows esptool.

`src/new_layout.rs`: the partition layout (§4.9), the size estimate (§2's measurements), the AP password, and file
hashes.

`app_config.rs`:
- the new questions: choice questions, the hardware question, and advanced questions (`--advanced`);
- old `--set` keys translated;
- reserved pins refused;
- the summary and confirmation.

`app_new.rs` and `project_update.rs`:
- `.raftnew.json`, and `--answers`;
- `raft update-project --set`, which re-renders the project: unedited files are updated, edited ones get
  `<file>.raftnew`.

Templates: the partition table, flash and PSRAM `sdkconfig`, `RAFT_NO_NETWORKING` and the runtime decoding settings
in `features.cmake`, `SysTypes.json` sections by connectivity, the random AP password, and the device type set.

**Checked:**
- 144 RaftCLI tests;
- `raft detect` on two ESP32-S3 boards under WSL (8 MB + 2 MB PSRAM, 16 MB + 8 MB PSRAM);
- the interactive flow through a pseudo-terminal, with detection: 12 prompts and the summary;
- builds of generated apps (below).

**Generated and built (ESP-IDF 6.1, branches' RaftCore/RaftSysMods/RaftI2C/RaftWebServer via `raftdevlibs`):** all
built, with only the LittleFS `lfs_mlist_isopen` warning. Estimates are those the summary printed.

| App | Built | Estimate |
|-|-|-|
| Defaults (ESP32-S3 4 MB, WiFi + BLE, common set) | 1,613,616 | 1.54 MB |
| Module ESP32-S3-WROOM-1-N8R8, no networking, no OTA, extended set, code decoding | 654,128 | 633 KB |
| Board FeatherS3, BLE only | 898,432 | 870 KB |
| ESP32-C6, WiFi only, all device types | 1,659,536 | 1.36 MB before, so RISC-V chips now use a factor of 1.16 (one measurement) |

**Still to do:**
- flash and run generated apps on boards: PSRAM settings, no-networking and BLE-only builds;
- check the board and module data marked "verify" against documentation;
- release RaftCore and RaftSysMods before RaftCLI, since apps made by the new `raft new` need `@common` and
  `RAFT_NO_NETWORKING` from their `main`.
