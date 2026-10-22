# RaftCLI native transports: plan

Date: 2026-10-18
Status: proposed, not started

## 1. Goal

RaftCLI talks to a running device only over plain HTTP (`raft fs`, `raft devtypes push/pull/status`, `raft ota`)
and has a text-only serial monitor. Raft devices also speak RICREST - REST commands, file transfer and firmware
update over any channel (serial, WebSocket, BLE). The WebUI and raftjs use it on every channel already.

This project gives RaftCLI a RICREST stack in Rust so the device commands work over serial, WebSocket and BLE as
well as HTTP:

- devices with no network (e.g. the Axiom over USB) can be managed;
- files move with flow control and a CRC check on any channel, and `raft fs` gets one file layer for all of them;
- the serial monitor can run file and REST commands on the port it already has open;
- large REST responses over serial no longer get lost (the text console drops responses of about 8 KB or more;
  framed responses don't have that limit).

## 2. What exists

### On the device (RaftCore, RaftSysMods, RaftWebServer)

- **RICREST** (`ProtocolExchange`): REST commands - the same strings as `/api/...` over HTTP - with responses
  matched by message number, on every registered channel.
- **File transfer** (`FileStreamSession`, `FileUploadOKTOProtocol`, `FileDownloadOKTOProtocol`):
  - upload: `ufStart` (fileName, fileLen, batchMsgSize, batchAckSize), file blocks, acks carrying `okto` (the
    position the device has accepted up to), `ufEnd`, `ufCancel`;
  - download: `dfStart`, blocks from the device, `dfAck` with `okto`, `dfEnd`, `dfCancel`;
  - `fileType` `fs`/`file` (a file, opened through `FileSystemChunker`, so any file can be read), `fw`/`ricfw`
    (firmware - an OTA update over any channel), or a real-time stream.
- **Framing:**
  - serial: `ProtocolRICSerial` (HDLC with CRC16-CCITT). SerialConsole carries these frames **on the same port as the
    text console**, encoded by `ProtocolOverAscii` as bytes with the top bit set, so text and frames interleave
    without a mode switch;
  - WebSocket: RICREST frames as binary WebSocket messages (RaftWebServer, `CommandSocket`);
  - BLE: RICREST frames on the BLE GATT service (`BLEGapServer`).

### In raftjs (the reference implementation)

- `RaftMiniHDLC.ts` (framing), `RaftMsgHandler.ts` (message numbering, request/response matching, timeouts),
  `RaftFileHandler.ts` (`fileSend` / `fileReceive`: the upload and download flows above, batching and acks),
  `RaftStreamHandler.ts`;
- channels: `RaftChannelWebSocket`, `RaftChannelBLE` (web and native), `RaftChannelWebSerial`,
  `RaftChannelSimulated` (for tests);
- used by the WebUI on all these channels, so the protocol is proven. raftjs is the specification for this work.

### In RaftCLI

- `http_client.rs` (GET, multipart POST, GET with timeout), `app_fs.rs` (`raft fs` over the FileManager REST API),
  `app_devtypes.rs` (`push` / `pull` / `status` over HTTP), `app_ota.rs`;
- `serial_monitor.rs`, `native_terminal.rs`: a text console; no framing.

## 3. Design

Four layers, each testable on its own:

1. **Link** - a trait that sends and receives frames:
   - `SerialLink`: RICSerial frames inside the console stream (ProtocolOverAscii), sharing the port with the
     monitor's text; text bytes go to the console, framed bytes to the RICREST layer;
   - `WebSocketLink`: binary messages (`tungstenite`, blocking, fits the current code);
   - `BleLink`: the GATT service's command and response characteristics (`btleplug`);
   - `SimulatedLink`: a scripted device for host tests.
2. **RICREST** - message numbering, request/response matching, timeouts, publish messages passed on (or ignored),
   and reconnecting a link after the device restarts. A port of `RaftMsgHandler.ts`.
3. **File transfer** - `put_file(local, remote, progress)`, `get_file(remote) -> bytes`, and later `put_firmware`. A
   port of `RaftFileHandler.ts`: batch size and ack size negotiated in `ufStart`, `okto` handling, retries,
   cancel, CRC check at the end.
4. **Device** - what commands use:

   ```rust
   trait DeviceConn {
       fn rest(&mut self, cmd: &str) -> Result<serde_json::Value>;     // e.g. "devman/typefile/check?file=..."
       fn put_file(&mut self, local: &Path, remote: &str, progress: Progress) -> Result<()>;
       fn get_file(&mut self, remote: &str) -> Result<Vec<u8>>;
       fn wait_for_restart(&mut self, timeout: Duration) -> Result<()>;
   }
   ```

   An HTTP implementation wraps today's code (REST GET, multipart upload, `filesection` download). The RICREST
   implementation works on any link.

**Choosing the device:** one argument for every device command:

| Form | Link |
|-|-|
| `192.168.86.52`, `http://host[:port]` | HTTP (as now) |
| `ws://host[:port][/path]` | WebSocket |
| `serial:COM4`, `serial:/dev/ttyACM0[@baud]` | Serial |
| `ble:<name or address>` | BLE |

Bare IP addresses keep working, so existing scripts don't change.

**Commands that move onto `DeviceConn`:** `raft devtypes push/pull/status` (they only use the four operations),
`raft fs` (list, get, put, delete, the FSImage mirror), and later `raft ota` (firmware over RICREST). The serial
monitor gains commands for file transfer and REST over its open port.

## 4. Phases

| Phase | Work | Verify |
|---|---|---|
| **1** | `DeviceConn` with the HTTP implementation; `raft devtypes` and `raft fs` use it (no change in behaviour); the device argument parsed | Existing tests; FeatherS3 over HTTP as before |
| **2** | HDLC framing, RICREST layer, file layer; `SerialLink`; `serial:` for `devtypes` and `fs`; host tests against `SimulatedLink` using raftjs's test cases | Host tests; Axiom over USB serial: `devtypes push/pull/status`, `fs put/get/ls`; a REST response over 8 KB arrives whole |
| **3** | The serial monitor shares its port with the RICREST layer: text to the screen, frames to RICREST; monitor commands for file and REST | Axiom: transfers while logs are printing, with neither garbled |
| **4** | `WebSocketLink` (`ws:`) | FeatherS3 over WebSocket |
| **5** | `BleLink` (`ble:`), scanning (`raft ports --ble` or `raft ble scan`) | Windows, macOS, Linux each against the FeatherS3 |
| **6** | Firmware over RICREST: `raft ota` over any link | OTA over serial and BLE |

Phase 2 gives the most value (devices without a network, and the 8 KB limit). Phase 4 is small once 2 is done.

## 5. Platforms (BLE)

`btleplug` gives one async API over Windows (WinRT), macOS (CoreBluetooth) and Linux (BlueZ over D-Bus), so most
platform differences stay inside it. Still to handle:

- **Async:** `btleplug` needs a `tokio` runtime; the rest of RaftCLI is blocking. Keep the runtime inside `BleLink`
  behind the blocking `Link` trait.
- **macOS:** the terminal app is asked for Bluetooth permission on first use; document it.
- **Linux:** BlueZ must be running, and the user may need to be in the `bluetooth` group.
- **WSL2:** no Bluetooth; use the Windows build of RaftCLI.
- **Build:** a `ble` cargo feature (default on), so platforms where `btleplug` doesn't build can leave it out.
- **Throughput:** a few KB/s, fine for type files (64 KB or less) and single files, slow for whole FSImages or
  firmware.

## 6. Risks and things to check first

- **Restarts on USB serial:**
  - native USB (USB Serial/JTAG, e.g. ESP32-S3) - the port goes away and comes back on a restart, and may get a new
    name on some hosts;
  - USB-UART bridges - the port stays, but opening it can toggle DTR/RTS and reset the board.
  `wait_for_restart` must reopen the port, and opening must leave DTR/RTS alone; test both kinds.
- **Which port carries RICSerial:** SerialConsole's `protocol` setting (default `RICSerial`). Check each target's
  SysType, e.g. which Axiom port has it.
- **File paths:** uploads and downloads open files through `FileSystemChunker` with the default file system. Check
  how a path naming the file system (`/local/devlib.new.json`, `/sd/...`) is handled, and match raftjs.
- **Logs on the console port:** framed bytes have the top bit set, but UTF-8 log text does too. Check how
  `ProtocolOverAscii` and SerialConsole tell them apart, and handle it the same way.
- **One link at a time:** a serial port can't be open in the monitor and in another `raft` process. Phase 3 solves
  this inside the monitor; until then, device commands need the monitor closed.
- **Versions:** older firmware without RICREST on a channel. Detect a missing response quickly and say so.

## 7. Not in scope

- A GUI.
- Running raftjs through Node from RaftCLI (it would add a Node dependency to a standalone binary). raftjs is the
  reference, not a dependency.

## 8. Related

- RaftCore `devdocs/device-type-record-management-plan.md`: `raft devtypes` (phase 4 of that plan, HTTP only). Its
  serial transport is delivered by this plan.
