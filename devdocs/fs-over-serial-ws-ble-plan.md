# `raft fs` over serial, WebSocket and BLE: plan

Date: 2026-10-22, updated 2026-10-25
Status: phases A to E done and merged (RaftCLI PR #2, 2026-10-25); BLE is tested on Windows only. Phase F isn't
planned (§9).
- The RaftSysMods fixes in §8 are on RaftSysMods `main` (`0753ed0`, and PR #6), and the RaftCore fixes are merged
  (PRs #14 and #21).
- btleplug comes from the fork `robdobsn/btleplug` until its WinRT fix is released (deviceplug/btleplug#507).
- All the decisions in §10 are made.

This plan takes the `raft fs` part of [native-transports-plan.md](native-transports-plan.md) and makes it concrete. That plan
covers the layers (link, RICREST, file transfer, device) and their phases. This one covers how each `raft fs` operation
maps onto RICREST, what the device firmware actually does (checked against the source on 2026-10-22), and BLE in
detail. Where the two disagree, this one is newer.

## 1. Goal

Every `raft fs` command works the same on all four links:

```
raft fs ls     192.168.86.52              # HTTP (as now)
raft fs ls     ws://192.168.86.52         # WebSocket
raft fs put    serial:/dev/ttyACM0 web/index.html
raft fs get    ble:Axiom-1234 config.json
raft fs        serial:COM4 -s MySysType   # mirror the built FSImage
```

`raft devtypes` uses the same file functions (`upload_file`, `download_file`, `delete_remote`), so it gets the new
links at the same time.

## 2. What `raft fs` does now

[src/app_fs.rs](../src/app_fs.rs) uses only the FileManager REST API over HTTP:

| Command | Device calls |
|-|-|
| `ls`, `df` | GET `/api/filelist/<fs>/<folder~sub>` |
| `get` | GET `/api/filesection/<fs>/<path>/<start>/4096`, repeated (base64 in JSON) |
| `cat` | GET `/api/fileread/<fs>/<path>` (raw text) |
| `put` | POST `/api/fileupload` (multipart; ignores `--fs`, writes to the device's default FS) |
| `rm` | GET `/api/filedelete/<fs>/<path>` |
| `sync` / default mirror | `filelist` (recursive), `filehash` per file, then `filedelete` and `fileupload` |
| `format` | GET `/api/reformatfs/<fs>/force` |

**Every operation except upload is a GET.** Over RICREST each one is the same string without `/api/`, sent as a
RICREST URL element, and the device replies with the same JSON. So once a link can carry RICREST REST commands,
everything but `put` works without further changes. Only upload needs the file-transfer protocol.

## 3. What the device does (from the firmware source)

The details below are what the Rust code must match. raftjs (`RaftMiniHDLC.ts`, `RaftMsgHandler.ts`,
`RaftFileHandler.ts`, the channel classes) implements the same protocol and is the reference for behaviour.

### 3.1 Frames

- **RICREST frame:** `[msgNum][(msgType<<6)|protocol][elemCode][body]`.
  - msgType: 0 = command, 1 = response, 2 = publish, 3 = report.
  - protocol 2 = RICREST, so a command is `[n, 0x02, ...]` and its response is `[n, 0x42, ...]`.
  - elemCode: 0 = URL, 1 = JSON command or response (NUL-terminated), 3 = command frame (JSON), 4 = file block.
- **Matching:** a response carries the request's msgNum. msgNum 0 means unnumbered, and the device uses it for
  messages it sends on its own (timer acks, cancel notices, download blocks). Number requests 1 to 255, wrapping back to 1.
  If a request is resent with the same msgNum while its response is still pending, the device ignores it, so a
  retry can safely reuse the number.
- **HDLC** (used on all three links): `E7 | esc(payload) | esc(crcHi) | esc(crcLo) | E7`.
  - Escape byte `D7`: an escaped byte is sent as `D7` followed by the byte XOR `0x20`. Only `E7` and `D7` are escaped.
  - CRC-16/CCITT-FALSE, the same function as `crc16_ccitt` already in `app_fs.rs`.
  - Device receive limit (`MaxRxMsgLen`): 5000 bytes without PSRAM, 200000 with.
- **Serial only, OverAscii:** the HDLC bytes are re-encoded so every byte has its top bit set, and they share the port
  with the plain-ASCII console:

  | Byte | Sent as |
  |-|-|
  | `00-0F` | `85`, `(b^0x20)\|0x80` |
  | `10-7F` | `b\|0x80` |
  | `80-8F` | `8E`, `b^0x20` |
  | `90-FF` | `8F`, `b` |

  On receive, bytes `>= 0x80` go to the OverAscii decoder and then to the HDLC decoder. Bytes `< 0x80` are log text.
  Config: `SerialConsole.protocol`, default `RICSerial`.
- **WebSocket:** HDLC frames as binary messages at `ws://<host>/ws`, with no OverAscii and no prefix byte. Config:
  `WebServer.websockets[]`, `pfix` default `ws`, `pcol` default `RICSerial`. The RaftCLI template sets the same values.
  `pcol: "RICJSON"` never sends REST responses back, so it can't be used.
- **BLE:** HDLC frames written to the command characteristic. Responses arrive on the response characteristic, as
  indications by default. A frame can span several writes and notifications. Details in §6.

### 3.2 FileManager over RICREST

- All the GET endpoints (`filelist`, `filesection`, `filehash`, `filedelete`, `reformatfs`, `fileread`) can be called
  over RICREST. `fileupload` (POST) cannot: the device replies "unknown API".
- Paths in REST strings use `~` for `/` (`encode_device_path` already does this).
- **`fileread` sends no response when it fails** on firmware without the §8 fix. The handler returns an empty
  string, and RICREST sends nothing for an empty response (`FileManager.cpp:209`, `ProtocolExchange.cpp:314`), so
  the client only sees a timeout.
  - Fixed firmware replies `{"rslt":"fail","error":"nofile"}`.
  - Either way, its success reply is raw file text inside a JSON element, not JSON.
- **`filesection` has no size cap.** The reply is about 1.33 × `len` plus around 100 bytes of JSON, so the client
  must choose `len` to fit the link (§5.3).

### 3.3 Upload (`ufStart` / file blocks / `ufEnd`)

- `ufStart` (elemCode 3, JSON): `{"cmdName":"ufStart","reqStr":"fileupload","fileType":"fs","fileName":..,"fileLen":N,"batchMsgSize":B,"batchAckSize":K}`.
  The reply gives the block size, ack batch size and `streamID` that are actually used. Always use the values from
  the reply.
- **Block size is capped per link:** at most `inboundBlockLen × 2/3`, which is **800 bytes** on serial and WebSocket
  (1200 default) and **333 bytes** on BLE (`maxPktLen` 500). `block × batch` is capped at 50000.
- **Block:** `[0x00, 0x02, 0x04, streamID, pos>>16, pos>>8, pos, data...]`. The position is 24 bits, which limits a
  file to 16 MB (fine for a filesystem).
- **Acks:** `okto` (the position accepted up to) is sent after the first block, after every `batchAckSize` blocks,
  and after the last block. A block at the wrong position is dropped but still counts toward the batch.
  - If no blocks arrive, the device sends timer acks: after 5 s for the first two blocks, after 1 s from then on, up
    to 5 times. After that it cancels the upload (`ufCancel`, reason `failRetries`).
  - The flow, as in raftjs: send the first batch as a single block, then batches of `batchAckSize`; wait for `okto`;
    resume from `okto` (go-back-N).
- `ufEnd` replies ok. The session normally closes itself after the last block, and a `ufEnd` that finds no session
  still gets ok.
- **The device does not check a CRC on file uploads.** `ufStart` accepts `CRC16`, but `writeFileBlock` ignores it.
  Check the upload afterwards with `filehash`.
- **Filesystem:** if `fileName` contains `local/` or `sd/`, it is used as an absolute path. Otherwise the device's
  default FS is prepended. **Always send `/<fs>/<path>`** so `--fs` is honoured. That is an improvement on HTTP
  upload, which ignores `--fs`. It also avoids a device quirk: the check looks for `local/` anywhere in the name, so
  `backup/nonlocal/x` is treated as an absolute path (`FileSystem.cpp:629`). Parent folders are created automatically.
- **Sessions:** at most about 4 at once. A session with no traffic for 10 s is dropped, and a repeat `ufStart` for a
  file that is already partly written fails with `transferInProgress`. Uploads are sequential, so this only matters
  after an aborted run: send `ufCancel` on Ctrl-C and on errors.

### 3.4 Download (`dfStart`): later, not needed at first

- `dfStart`, `dfAck` (`okto`, no reply), and `dfEnd`. The device sends at most one block every 100 ms: up to
  5000 bytes per block on serial and WebSocket (about 50 KB/s), and 500 bytes on BLE (about 5 KB/s).
- The session closes itself as soon as `okto` reaches the file size. A `dfEnd` sent after that gets a plain ok
  **without the `crcAt=end` CRC**, so it would have to be verified with `filehash` anyway.

`filesection` already works on every link and is simpler, so `get` starts with it (§5.3). `dfStart` becomes worth
doing only if `filesection` turns out too slow on a link.

## 4. Design

### 4.1 One connection type for all commands

As built in phase A ([src/device_conn.rs](../src/device_conn.rs)):

```rust
pub trait DeviceConn {
    fn rest_raw(&self, cmd: &str) -> CmdResult<Vec<u8>>;      // "filelist/local/web" -> the reply body
    fn put_file(&self, local: &Path, remote: &str, fs: &str, progress: &mut ProgressFn) -> CmdResult<()>;
    fn wait_for_restart(&self, timeout: Duration) -> bool;     // after devtypes apply (and later format)
    fn describe(&self) -> &str;                                // the device as given: messages, per-device records
    fn link_kind(&self) -> LinkKind;                           // filesection chunk size; later timeouts
    // Provided:
    fn rest(&self, cmd: &str) -> CmdResult<serde_json::Value>;     // rest_raw + parse
    fn rest_ok(&self, cmd: &str) -> CmdResult<serde_json::Value>;  // + "rslt":"ok", else "Device error: <error>"
}
```

- **Methods take `&self`.** Commands pass `&DeviceCtx` around, and `DeviceCtx` holds the connection in an `Rc` so
  `with_fs` can share it. Links with state (a reader thread, message numbers) keep it behind interior mutability.
- **No timeout parameter yet.** HTTP uses its 30 s. A RICREST link will need a longer timeout for `reformatfs`; add a
  provided `rest_raw_timeout` then.
- **`HttpConn`** puts today's code behind the trait:
  - `rest_raw` is `http_get("/api/" + cmd)`, returning the body whatever the HTTP status, as callers read the
    device's JSON;
  - `put_file` is the multipart POST, which still ignores `fs`;
  - `wait_for_restart` polls `v`.
- **`RicrestConn<L: Link>`** (phase B) implements `rest_raw` as a URL element and `put_file` as `ufStart`, blocks and
  `ufEnd`.
- `DeviceCtx { conn: Rc<dyn DeviceConn>, fs }`. The parsing and sync logic didn't change.

### 4.2 Choosing the device

The positional `IP_ADDRESS_OR_HOSTNAME` argument becomes `DEVICE`, parsed by `parse_device_spec`:

| Form | Link |
|-|-|
| `192.168.86.52`, `host`, `http://host[:port]` | HTTP; a bare IP or host works as before |
| `ws://host[:port][/path]` | WebSocket; the path defaults to `/ws` |
| `serial:/dev/ttyACM0`, `serial:COM4`, `serial:` (auto-select, as `raft monitor` does) | Serial; `--baud`, default 115200 |
| `ble:<name, name prefix or address>`, `ble:` (the only one found) | BLE |

- `-p/--port` stays the HTTP port. It is not reused for the serial port: the monitor's `-p` means something else,
  and the port name now goes in `DEVICE`. A port in both `DEVICE` and `-p` that differ is an error.
- **`-d/--device <DEVICE>`, defaulting to `RAFT_DEVICE`** (decision 4, `resolve_device`):
  - The device is `-d` if given, else `RAFT_DEVICE` if set, else the first positional argument.
  - With `-d` or `RAFT_DEVICE`, the device is left out of the positionals: `raft fs get config.json`. Without them,
    commands are unchanged: `raft fs get 192.168.86.52 config.json`.
  - So clap sees every positional as optional (usage `raft fs get [DEVICE] [REMOTE] [LOCAL]`). Required ones are
    marked "(required)" in the help and checked after the device is taken.
  - `RAFT_DEVICE` is read in code, not with clap's `env`. `raft fs` rejects subcommands when its mirror options are
    given, and a value from the environment could count as given.
  - With `RAFT_DEVICE` set, a device given positionally is taken as the command's first argument (for example
    `raft fs ls 192.168.86.52` lists a folder of that name). The help says to leave the device out in that case.

### 4.3 RICREST layer (`src/ricrest/`)

As built in phase B:

| File | Contents |
|-|-|
| `hdlc.rs` | Encoder and streaming decoder; frames with a bad CRC (or over 256 KB) are counted, then dropped |
| `overascii.rs` | Encoder and streaming decoder; bytes below 0x80 go out as text |
| `frame.rs` | `Frame::parse`, `text_frame` (URL or JSON, NUL-terminated), `file_block` |
| `session.rs` | `Session<L: Link>`: `request(elem, text, policy) -> Frame`, `send(frame)`, `wait_for(pred, timeout)` |
| `upload.rs` | `upload(session, file_name, data, sizes, policy, progress)`: the flow from §3.3 |
| `conn.rs` | `RicrestConn<L: Link>`: `DeviceConn` over a link, with request policies and upload sizes per link kind |
| `tests.rs` | Protocol tests against the simulated device |

The CLI is sequential, so `session.rs` is simpler than raftjs:

- **One outstanding numbered request at a time**, numbered 1 to 255 and back to 1 (0 is never used).
- `request` waits until the response with its msgNum arrives, and **resends the same frame** on timeout:
  - 2 s × 3 on WebSocket, 3 s × 3 on serial and BLE;
  - `reformatfs` will need longer (phase C).
- Frames nobody is waiting for are dropped: late and duplicate responses (counted in `stale_responses`), and
  publish and report frames.
- **Upload** (`upload.rs`):
  - acks are taken from unnumbered responses that have `okto`;
  - a `ufCancel` from the device, or a `ufStatus` failure, ends the upload with the device's reason;
  - after any error once `ufStart` has succeeded, `ufCancel` is sent, so the next upload doesn't get
    `transferInProgress`;
  - it gives up after 6 waits or acks in a row without progress, waiting 10 s for each ack;
  - files of 16 MB or more are refused, as the block position is 24 bits.
- **Upload resend rule.** Each batch starts at the latest ack (go-back-N). After waiting for an ack, every ack
  already received is read and the furthest one counts, so no ack is ever ignored.
  - RaftCore PR #14 (merged 2026-10-23) acks the first block out of sequence at once, and keeps its acks in step
    with the host's batches. With that firmware this rule resends little and doesn't wait for the device's timer.
  - Older firmware counts the blocks it drops after a loss, so its acks fall mid-batch and this rule resends more,
    but it doesn't stall.
  - Simulated, 200 blocks with 6 lost:
    - current firmware: 239 blocks sent (losses mid-batch) or 251 (at the start of a batch), 0 to 1 timer acks;
    - older firmware: 367 to 389 blocks, 1 timer ack.
    - The raftjs test harness gives the same figures for the same rule.
  - **Rejected alternative: carry on past a partial ack.** It is better with older firmware (about 300 blocks), but
    with current firmware it sends blocks the device drops without an ack, then waits 1 s for a timer ack after
    every mid-batch loss: 12.9 s against 5.4 s in the raftjs harness. Timer acks can't be told apart from other acks
    by their content: a file block's `req` is `"ufBlock"` too.
  - raftjs PR #17 used that alternative; raftjs PR #18 (merged 2026-10-23) changed it to this rule.
- `RicrestConn::put_file`:
  - sends `fileName` as `/<fs>/<path>`;
  - checks the result with `filehash`. On a mismatch it deletes the file and fails.
- On serial, console text goes to stderr with `--show-log`, and is dropped otherwise (phase C).
- **Strings:** send UTF-8. raftjs sends Latin-1, which matters only for non-ASCII file names. Note this, and match
  raftjs if the device turns out to need it.

### 4.4 Link trait (`src/link/`)

As built:

```rust
pub trait Link {
    fn send(&mut self, frame: &[u8]) -> CmdResult<()>;                        // a RICREST frame; the link encodes it
    fn recv(&mut self, timeout: Duration) -> CmdResult<Option<Vec<u8>>>;      // next frame; zero timeout = no wait
    fn bad_frames(&self) -> u32 { 0 }                                         // CRC failures, for -v
    fn reopen(&mut self, timeout: Duration) -> bool { false }                // after a device restart
}
```

- `recv` replaces the planned `frames() -> &Receiver`. A link with a reader thread wraps its channel in `recv`, and
  the simulator needs no thread.
- Links take whole RICREST frames and do their own wire encoding, with `encode_wire` and `WireDecoder`:
  - serial: OverAscii(HDLC), with the text between frames passed out;
  - WebSocket: HDLC per message;
  - BLE: HDLC split into chunks of the write size.
- **`SimulatedLink`** (`link/sim.rs`, tests only):
  - frames go through the real wire encoding both ways, into a `SimDevice`;
  - REST answers come from `FakeConn`'s file tree;
  - the upload protocol works as `FileUploadOKTOProtocol` does:
    - with current RaftCore rules (the default) or earlier ones (`ack_out_of_sequence = false`);
    - every ack is `{"req":"ufBlock",...}`, as on the device;
    - the first-block ack, `ufStart` clamping (2/3 of 1200 or 500, and 50000 bytes per batch), and
      `transferInProgress`;
    - timer acks (counted, so tests can check none were needed), and a cancel after 5 of them;
  - faults can be switched on: lost blocks, all blocks lost, a lost reply, duplicate replies, a corrupted reply,
    log text before and inside frames, a device-side cancel, and a damaged upload;
  - time doesn't pass. A wait with nothing due gets what the device's timer would send, or ends at once.

## 5. Per-command behaviour over RICREST

### 5.1 No change beyond `conn.rest`

`ls` (including `-r` and `--json`), `df`, `rm`, `sync`'s listing, hashing and deleting, and `format`.

- `format` restarts the device. On serial (native USB) and BLE the link drops, so treat a lost link after
  `reformatfs` as success and say so.

### 5.2 `put`

- `RicrestConn::put_file` runs the upload flow from §3.3, then calls `filehash` and compares the result with
  `local_crc16`. On a mismatch, delete the file and fail.
- Progress uses the same `\r` line as HTTP.
- Ctrl-C sends `ufCancel`, so the next run doesn't hit `transferInProgress`.
- Requested sizes; the device clamps them and its reply decides:

  | Link | Block | Batch |
  |-|-|-|
  | Serial | 800, cut to fit 900 bytes on the wire | 1 (§7.2) |
  | WebSocket | 800 | 10 |
  | BLE | 333 | 20 (40 was only 6% faster; measured in §6.3) |

### 5.3 `get` and `cat`

- `download_file` keeps the `filesection` loop with a chunk size per link:

  | Link | Chunk | Why |
  |-|-|-|
  | HTTP | 4096 | as now |
  | WebSocket | 4096 | |
  | Serial | 1024 | Firmware without the §8 fix encodes each reply in a stack buffer of 2 × the frame length, so a 4 KB chunk (about 5.6 KB of reply) risks a stack overflow. Raise it once the fix is widespread |
  | BLE | 4096 | About 9 indications per reply. Measured 8.3 KB/s, against 6.1 at 1024; larger chunks don't help (§6.3) |

- Then call `filehash` and compare, once per file rather than per chunk.
- **`cat` uses `download_file` on every link** (decision 2, done in phase A).
  - On older firmware, `fileread` hangs until timeout over RICREST when the file is missing (§3.2), and it isn't
    binary-safe on any firmware.
  - This also removes the HTTP binary warning, and `fileread` is then no longer used.

### 5.4 Mirror / sync

- No logic changes.
- Over BLE, a full FSImage of a few hundred KB uploads at about 20–30 KB/s (measured on Windows), so it takes
  10–30 s. Print an estimate before starting (sum of upload sizes ÷ the link's typical rate).
- Hashing makes one `filehash` round trip per file. That is fine on every link: about 30–45 ms each over BLE
  (measured). Keep it.

## 6. BLE in detail

RaftCLI has no BLE support today. Everything below is new.

The Windows findings below were measured with a spike (btleplug 0.11.8 and 0.13.5) against an Axiom
(ESP32-S3, PSRAM, firmware 0.1.4) on 2026-10-22. The results are in §6.5. macOS and Linux are still untested.

### 6.1 Crate and threading

- **Crate:** `btleplug` **0.13** (0.13.5 is current, and `Peripheral::mtu()` was added after 0.11). It uses WinRT on
  Windows, CoreBluetooth on macOS and BlueZ/D-Bus on Linux, and is the only maintained cross-platform central-role
  crate.
- **Unmodified btleplug can't open the Raft service on Windows** (§6.5, finding 1). The fix is one call in btleplug's
  WinRT backend. Until it is upstream, use a fork through `[patch.crates-io]`.
- **Threading:** `btleplug` is async and needs `tokio`. `BleLink` owns a current-thread runtime on its own thread:
  - commands go in over a channel;
  - notifications are HDLC-decoded and pushed into the `frames()` channel;
  - the rest of RaftCLI stays blocking.
- **Build:** add a `ble` cargo feature, on by default.
  - On Linux, `btleplug` needs `libdbus-1-dev` and `pkg-config` at build time, so the release workflow's Linux job
    needs that package.
  - Builds that leave the feature out reject `ble:` with a clear message.

### 6.2 Finding the device

- **Service UUID.** Two defaults are in use:
  - `aa76677e-9cfd-4626-a510-0d305be57c8d` (`…8e` command, `…8f` response) is the RaftSysMods library default, and the
    first one raftjs tries;
  - `bb76677e-…`, which the RaftCLI `raft new` template wrote until 2026-10-23. The template now writes `aa76…`
    (decision 1), but apps made before then still advertise `bb76…`.
  - raftjs also accepts `da903f65-d5c2-4f4d-a065-d1aade7af874`.
  - Scan for all three, and allow overrides with `--ble-service`, `--ble-cmd` and `--ble-resp`.
  - The service UUID is in the advertisement, and the name is in the scan response, a separate packet.
  - **Don't use btleplug's `ScanFilter` on Windows** (§6.5, finding 4):
    - with several UUIDs it matches nothing;
    - with one UUID it drops the scan response, so the name is never seen.
  - Scan unfiltered. Take the address from any packet that carries a Raft UUID, and the name from the same address.
- **Name:** `BLEMan.adName`, falling back to the friendly name, then the system name. It is in the scan response and
  truncated to fit, so match `ble:<x>` as an exact name first, then as a prefix, then as an address. The Axiom
  advertises `Axiom018_2cc23e` and was found 0.5–2.3 s after the scan started.
- **New command:** `raft ble scan [--timeout 5]` lists name, address, RSSI and service UUID for Raft devices. Better
  as its own command than `raft ports --ble`, because the scan is slow and async, unlike port listing.
- **Cache:** cache the last address in `~/.raftcli/` (or wherever RaftCLI keeps state), keyed by name. Connecting to
  a known address skips a 2–5 s scan on macOS and Windows, where scans are slow.

### 6.3 Data path

- **Writes:** use write-without-response to the command characteristic, in chunks of the write size.
  - Write size = `mtu() - 3`, from btleplug 0.13's `Peripheral::mtu()`. Fall back to 182 (as raftjs web does) if it
    reports the 23-byte default, and allow `--ble-mtu` to override.
  - Read `mtu()` after subscribing. In some runs it reported 256 while 512 was in use. A low value only means
    smaller writes, so it's safe.
  - Windows negotiates MTU 512 with the device. Writes of up to **509 bytes** work. Larger writes fail with
    `E_INVALIDARG`, so an oversized write is an error, not silent truncation.
  - At 509 bytes, a 333-byte upload block is one write.
  - No delay is needed between writes. On Windows, 0 ms was faster than 2 ms and lost nothing.
- **Responses:** subscribe to the response characteristic. It declares READ, NOTIFY and INDICATE. The device sends
  **indications** by default (`sendUseInd`), one at a time.
  - **After `subscribe`, always write `[0x01, 0x00]` (notify) to the CCCD descriptor** with `write_descriptor`.
    That is public API, so no fork is needed.
    - On Windows, btleplug writes **indicate only** (`0x0002`) for a characteristic that supports both.
    - Firmware without the §8 fix counts a subscription only from the notify bit (`BLEGattServer.cpp:542`), so it
      never replies (§6.5, finding 2).
    - Fixed firmware accepts either bit.
    - Writing notify works on both, and was tested on both on 2026-10-23, so there's no need to detect the version.
    - On Linux, BlueZ chooses notify when both are supported anyway. Still check this, and check macOS.
  - The device then sends indications anyway (NimBLE doesn't check the CCCD), and Windows accepts and confirms
    them. Keep it that way:
    - with `cmdUseInd=0` (notifications), replies of 4 KB or more lost a packet (an HDLC CRC failure), and the
      request timed out;
    - for 1 KB replies, notifications were no faster (7.4 against 6.1 KB/s).
- **Disconnect: always send `bledisconnect`, on every exit path, including errors and Ctrl-C** (§6.5, finding 3).
  - Windows keeps the BLE link up after btleplug's `disconnect()` and after the process exits. It was still up after
    more than 3 minutes.
  - While connected, the device doesn't advertise, so no client can find it until it is released or restarts. The
    Axiom also pauses WiFi while BLE is connected.
  - **Update (2026-10-25):** with RaftSysMods 1.26 or later (`BLEConnArbiter`, two connections) the device keeps
    advertising while one link is held, and WiFi pausing follows the session. A RaftCLI that is killed (so it can't
    send `bledisconnect`) is released by Windows about 25 s later, and a new run meanwhile reuses the held link.
    Details: RaftCore `devdocs/ble-held-links-windows.md`. `bledisconnect` is still sent on every exit.
  - Retry `bledisconnect` up to 3 times. Once, the device was still connected after a run that had sent it, so a
    single attempt isn't enough.
  - If RaftCLI ever finds a device it can't connect to, say so: the fix is `bledisconnect` over another channel, or
    a restart.
- **Measured throughput** (Windows, Axiom, MTU 512):

  | Operation | Settings | Rate |
  |-|-|-|
  | Upload 50 KB | block 333, batch 10, 182-byte writes, 2 ms gap | 16.5 KB/s |
  | Upload 50 KB | block 333, batch 10, 509-byte writes, no gap | 21.2 KB/s |
  | Upload 200 KB | block 333, batch 20 | 27.7 KB/s |
  | Upload 200 KB | block 333, batch 40 | 29.5 KB/s |
  | Download 100 KB, `filesection` | chunk 1024 | 6.1 KB/s |
  | Download 100 KB, `filesection` | chunk 4096 | 8.3 KB/s |
  | Download 100 KB, `filesection` | chunk 16384 | 8.1 KB/s |
  | REST round trip (`v`, `filelist`) | | 30–45 ms |

  - Uploads had no short acks and the `filehash` CRC matched every time.
  - Downloads are limited by indications (about one every 40 ms, around 480 bytes each). Raising the chunk size above
    4096 doesn't help. `dfStart` (one 500-byte block every 100 ms, about 5 KB/s) would be slower.
  - Connection setup: connect 0.4–1.4 s, service discovery 0.3–0.5 s, subscribe 40–60 ms. With the scan, that is
    2–4 s before the first command.
  - **Rates vary from day to day.** On 2026-10-23, the same tests on the same firmware gave:
    - uploads of 16–27 KB/s;
    - downloads of 4.3–6.3 KB/s;
    - uploads of about 14 KB/s with the serial monitor open on the device's USB port.

    Estimates (§5.4) should use the low end: 15 KB/s up, 5 KB/s down.

### 6.4 Platforms

| Platform | Notes |
|-|-|
| Windows | WinRT. No pairing needed. Tested. Needs the btleplug fix, the CCCD write (§6.3) and `bledisconnect` (§6.5). |
| macOS | The terminal app asks for Bluetooth permission the first time. Addresses are per-host UUIDs, not MACs, so match by name. |
| Linux | BlueZ must be running; the user may need to be in the `bluetooth` group. |
| WSL2 | No Bluetooth. Hand the command to `raft.exe`, as for serial (§7.1). |

### 6.5 Windows spike results (2026-10-22)

Setup:

- Windows 11 (build 26200), Intel Wireless Bluetooth, Rust 1.91 (MSVC), run from WSL2 through the Windows toolchain;
- an Axiom (ESP32-S3) running RaftSysMods firmware 0.1.4 with the library-default service `aa76…`;
- the spike: one Rust file covering scan, GATT listing, RICREST over HDLC, a single-write size probe, a `filesection`
  download loop and a `ufStart` upload with `filehash` check. It is kept outside the repo.

BLE over RICREST is viable on Windows. File transfer is fast and reliable once these four issues are handled:

1. **The Raft service's characteristics are refused (`AccessDenied`).**
   - btleplug's `GetCharacteristicsWithCacheModeAsync(Uncached)` returns `GattCommunicationStatus::AccessDenied` for
     the `aa76…` service, every time, in 0.11.8 and 0.13.5. The standard GAP and GATT services are fine.
   - btleplug logs the status at debug level and returns an empty list, so the service appears to have no
     characteristics.
   - **Fix:** call `service.OpenAsync(GattSharingMode::SharedReadAndWrite)` before fetching characteristics. With
     it, every run worked (about 20 over the session); without it, 0 of 6.
   - `RequestAccessAsync` alone doesn't help (0 of 2): it returns `Allowed` and characteristic discovery is still
     refused.
   - It is a small patch in `winrtble/ble/device.rs` (`get_characteristics`). Send it upstream, and use a fork
     meanwhile.
   - **It isn't specific to btleplug.** `bluest` 0.6.9 gets the same `AccessDenied`, and reports it as
     `NotAuthorized`. Its UUID-filtered service discovery also fails with `0x80070016` ("the device does not
     recognize the command"). Any WinRT client needs the `OpenAsync` call. The device may be the cause, so an
     ATT-level trace is worth doing. Windows Bluetooth ETW logging or an nRF sniffer would show the request that is
     refused.
2. **Indicate-only subscription gets no replies.** See §6.3. btleplug writes CCCD `0x0002`, and the firmware ignores
   it. Writing `0x0001` (or `0x0003`) afterwards fixes it. Fix the firmware too (§8).
3. **Windows doesn't release the link** (with `bluest` too).
   - After btleplug disconnects, or the process exits, the device stays connected for more than 3 minutes and
     doesn't advertise.
   - `bledisconnect` releases it within 200 ms.
   - If RaftCLI crashes without sending it, the device can't be reached over BLE until it is released another way
     (for example, typing `bledisconnect` on the serial console) or restarted.
   - Retested on 2026-10-25 with RaftSysMods 1.27 and RaftCLI's BLE link: Windows ended the link of a killed
     RaftCLI about 25 s after it died (3 of 3 runs), and the device kept advertising meanwhile (§6.3, update).
4. **btleplug's `ScanFilter` is unusable on Windows.** See §6.2.

5. **Windows sometimes reconnects to the device by itself.**
   - Seen twice on 2026-10-23, each time on the second run after the device had been reflashed, with both the old
     and the fixed firmware.
   - About 3 s after a clean `bledisconnect`, something on Windows connected and held the link. The next run's scan
     then didn't find the device.
   - It didn't happen when the device was idle, or across four consecutive runs on a device that hadn't been
     reflashed.
   - Treat it like finding 3. If a device that should be in range isn't found, say it may be held by another
     connection, and that `bledisconnect` over serial or WiFi releases it.

Things that just worked:

- MTU 512 negotiated by Windows; `mtu()` (0.13) reports it;
- write-without-response of up to 509 bytes;
- HDLC frames split across writes and indications;
- the frame format in §3.1: HDLC with CRC-16/CCITT-FALSE, a trailing NUL on URLs, and the 7-byte file block header.
  The §9 test vectors weren't compared byte for byte, but the device accepted every frame from the same encoder;
- `ufStart` clamping the block to 333;
- `filesection` and `filehash` over RICREST;
- no pairing needed.

**With the fixed firmware (2026-10-23).** The §8 RaftSysMods fixes were built for the Axiom against the local
RaftSysMods, flashed, and tested with the spike:

- Replies arrive with CCCD `0x0001`, `0x0002`, `0x0003` and btleplug's default.
- `fileread` of a missing file returns `nofile` in about 30 ms.
- Replies go out in about 500-byte indications, sized from the connection's actual MTU.
- 200 KB and 100 KB uploads and a 100 KB download all matched `filehash`.
- Throughput was the same as the original firmware on the same day (A/B tested).

The first fixed build reset the MTU on connect and tracked it from the MTU event. That left replies in 11-byte
pieces (about 480 ms for `v`), because the reset wiped out the MTU learned from Windows' exchange. The final fix reads
`ble_att_mtu()` from NimBLE each time it sizes a packet.

## 7. Serial in detail

- **Opening without resetting the board:**
  - The monitor opens with `dtr_on_open(true)`. On boards with a USB-UART bridge and the usual auto-reset circuit,
    changing DTR or RTS on open can reset the board.
  - `SerialLink` opens without changing DTR or RTS, to match WebSerial.
  - Test on a bridge board and on a native-USB board (ESP32-S3).
- **Port in use:** the port can't be open in `raft monitor` and `raft fs` at once. Fail with "port in use, close
  the monitor". Phase 3 of the native-transports plan, file commands from inside the monitor, fixes this properly.
- **Log text on the same port:**
  - Bytes `< 0x80` are text; `>= 0x80` are frame data.
  - Non-ASCII UTF-8 in device logs also has the top bit set, so it corrupts the frame being decoded. The CRC catches
    it and the request is retried.
  - Count CRC failures (shown with `-v`), so a log-heavy device shows up as the cause of slow transfers.
- **Throughput:** on native USB the baud rate doesn't apply. Measured: uploads about 10-12 KB/s (set by flash
  writes and the device's receive limits) and downloads about 42 KB/s (§7.2).
- **Restarts:** native USB ports disappear and come back after `format`. Reconnecting isn't needed for `raft fs`;
  report it and stop.

### 7.1 WSL

WSL2 has no access to USB serial ports or Bluetooth. `raft flash` and `raft ports` already hand the whole command to
the Windows `raft.exe` (`app_flash.rs:114`), unless `-n/--native-serial-port` is given. `raft fs` and
`raft devtypes` should do the same for `serial:` and `ble:` devices:

- re-run the same arguments with `raft.exe` and stream its output;
- translate local paths for Windows with `wslpath -w`, because `put`, `get`, `sync` and the FSImage mirror read and
  write local files;
- keep HTTP and `ws:` running natively in WSL;
- the pushed-CRC record that `raft devtypes` keeps is per device and per project. It is written by whichever binary
  runs, so it must be in the project folder, not a per-user directory. It already is: `devtypes-pushed.json` in
  `Project::work_dir`.

### 7.2 Phase C results (2026-10-23)

Tested from WSL, through the Windows build of this branch (the hand-off in §7.1). The board was an Adafruit Feather
ESP32-S3 TFT: native USB, console on USB Serial/JTAG. It ran a throwaway app from `raft new` (generic ESP32-S3,
4 MB flash) with RaftCore and RaftSysMods `main`.

**Three problems found and fixed in RaftCLI:**

1. **On Windows, a reader thread made every write wait for seconds.**
   - Each `filesection` request took 0.4-9.6 s, all of it in writing the request. A 50 KB download took 88 s.
   - `serialport`'s `try_clone` duplicates the handle, and synchronous reads and writes on one Windows file
     object wait for each other.
   - A single-threaded test program did the same requests in 22 ms each.
   - `SerialLink` now reads only in `recv`, on the calling thread: the same 50 KB download takes 2.0 s.
2. **The device loses a serial frame that is longer than about 1 KB on the wire** (§8). That is two limits:
   - SerialConsole's receive buffer, 1024 bytes by default (USB Serial/JTAG drops what doesn't fit);
   - its channel queue: SerialConsole queues what it reads as messages of a USB packet or so each, in a queue of
     20 ("inboundQueueAdd QUEUE IS FULL").

   The fix: serial uploads send one block at a time (batch 1), and each block is cut so the frame is at most 900
   bytes on the wire. The host knows the exact encoding, and the device takes blocks of any length.
3. **Writes timed out part-way through a frame.** While the device writes a block to flash (about 50 ms), the USB
   holds the host's writes off. The port's single 20 ms timeout applied to writes too, so a write gave up and left
   half a frame on the wire. Writes now have a 5 s timeout of their own.
   - Before the fix: 800-byte blocks in batches of 4 failed outright with random data, and smaller sizes ran at
     about 1 KB/s, waiting for timer acks.
   - Data that takes 2 or 4 wire bytes a byte (00-0F, 80-FF, and E7 or D7) gets shorter blocks.
   - A simulator test sends random data, zeros, 0xFF and 0xE7 against a 1024-byte limit; nothing is lost.

**Measured** (default SerialConsole settings):

| Operation | Result |
|-|-|
| Upload 50 KB random | 4.4-5.0 s, 9.9-11.4 KB/s; 3.2 s, 15.8 KB/s with RaftSysMods PR #6 |
| Upload 50 KB zeros | 4.9 s, 10.3 KB/s |
| Upload 20 KB of 0xE7 (4 wire bytes each) | 3.0 s, 6.6 KB/s |
| Upload 200 KB random | 17.1 s, 11.7 KB/s (each block takes about 50 ms to write to flash) |
| Download 200 KB (`filesection`, 1 KB chunks) | 4.8 s, about 42 KB/s |
| `ls`, `df`, small requests | 0.15-0.7 s, including starting raft.exe |

With RaftSysMods PR #6, 800-byte blocks one or two at a time work (16.7-17.5 KB/s). RaftCLI keeps its default
(one block of at most 900 wire bytes), as that works with firmware without the PR too. Larger batches overflow even
the 4096-byte buffer for data that doubles on the wire, and the driver drops what doesn't fit (it has no flow
control), so batching wouldn't gain much.

**Also checked:**
- `ls -r`, `df`, `cat`, `get` and `put`, with local paths absolute in WSL (translated for Windows) and relative;
  `RAFT_DEVICE`; `-v`; `--show-log`.
- The FSImage mirror (dry run and real); `sync --hash` (a second run has nothing to do).
- `format`: the device replied before restarting.
- `devtypes status`, `push --dry-run` and `push`. The device restarted, the port was reopened, the device was
  polled, and the pushed CRC was recorded under `serial:COM10`. All 2.4 s.
- **An upload killed part-way and run again at once:** the device said `transferInProgress`, RaftCLI cancelled it
  by name, and the upload went through.
- **The port held open elsewhere:** "serial port COM10 is in use - close raft monitor (or whatever else has it
  open)".
- **No reset on open:** the device's log timestamps kept rising across separate runs. DTR isn't needed: opening
  with DTR on made no difference on this board.

**Not tested:** a USB-UART bridge board (DTR and RTS reset), Linux and macOS serial, and heavy log output during a
transfer. The device logged only every few seconds; `--show-log` showed it, with no bad frames.

**Found on the way, and fixed:** `raft devtypes` looked for RaftCore only under `build/<SysType>/_deps/raftcore-src`.
The build fetches it to `build/<SysType>/raft/RaftCore` (RaftCore's `RaftBootstrap.cmake`), so without a
`raftdevlibs` copy it needed `--raftcore`. `find_raftcore` in `app_devtypes.rs` now looks there too, and keeps
`_deps` for older builds.

**For tuning:** `RAFTCLI_UPLOAD_SIZES=<block>,<batch>[,<most wire bytes per block>]`.

## 8. Device-side issues found

These are firmware issues, outside RaftCLI, found during this investigation. None blocks the plan, because the plan
works around each one, but they are worth fixing:

RaftCLI has to work with firmware that has none of these fixes, so every workaround in this plan stays even after
the fixes ship.

| Issue | Where | Effect | Fix | Status |
|-|-|-|-|-|
| `fileread` failure sends no response | RaftSysMods `FileManager.cpp:209` | RICREST client hangs until timeout | Return `{"rslt":"fail","error":"nofile"}` | Done: RaftSysMods `0753ed0` |
| Encode buffer on the stack, 2 × frame length | RaftSysMods `SerialConsole.cpp:722` | A large `filesection` reply over serial can overflow the stack | Stack up to 1000 bytes, heap above | Done: RaftSysMods `0753ed0` |
| Indicate-only CCCD not registered as a subscription | RaftSysMods `BLEGattServer.cpp:542,545` | A client writing CCCD `0x0002` (btleplug on Windows) gets no replies | Subscribed if `cur_notify \|\| cur_indicate`; cleared on connect and disconnect | Done: RaftSysMods `0753ed0`, tested |
| Subscribe event for another handle updates the response state | RaftSysMods `BLEGattServer.cpp:544-546` | A subscription to another characteristic (e.g. Service Changed) can switch replies on or off | Only the response characteristic's handle counts | Done: RaftSysMods `0753ed0`, tested |
| MTU kept from the previous connection | RaftSysMods `BLEGattOutbound.cpp:49`, `BLEGapServer.cpp:610` | A client that doesn't exchange MTU gets 500-byte chunks, which NimBLE truncates | Size chunks from `ble_att_mtu()` of the current connection (new `BLEGattServer::getConnMTU()`) | Done: RaftSysMods `0753ed0`, tested |
| Upload acks out of step with the host's batches after a lost block; acks for every block after a lost second block | RaftCore `FileUploadOKTOProtocol.cpp` (`validateRxBlock`) | After a loss, raftjs resent blocks and waited 1 s per batch; RaftCLI resends more | Ack the first block out of sequence at once; don't count blocks out of sequence; restart the count on a timer ack | Done: RaftCore PR #14, merged 2026-10-23 |
| A serial frame of more than about 1 KB on the wire is lost | RaftSysMods `SerialConsole` | Serial uploads must go a block of up to 900 wire bytes at a time (§7.2): about 10-11 KB/s | Read everything available each loop into one message (not up to 100 bytes, one at a time), and a 4096-byte default `rxBuf` for USB Serial/JTAG | Done: RaftSysMods PR #6 (merged 2026-10-23): 15.8 KB/s with RaftCLI's default |
| No release of an idle link | RaftSysMods `BLEGapServer` | A client that leaves without `bledisconnect` (Windows holds the link, §6.5) keeps the device unreachable over BLE | Optional: disconnect after N s with no subscription and no traffic | Not needed: Windows keeps the subscription of a dead client, so it couldn't be detected that way, and since RaftSysMods 1.26 the device keeps advertising and Windows ends the link after about 25 s (RaftCore `devdocs/ble-held-links-windows.md`) |
| Upload CRC16 ignored | RaftCore `FileStreamSession.cpp:518-531` | No end-to-end check; the CLI must call `filehash` | Check `CRC16` when it is given | Open |
| `dfEnd` after auto-close loses the CRC | RaftCore `FileDownloadOKTOProtocol.cpp:401` | `crcAt=end` is unreliable | Keep the session until `dfEnd` or a short timeout | Done: RaftCore PR #21 (`aea92db`) |
| `local/` matched anywhere in a path | RaftCore `FileSystem.cpp:629` | `a/nonlocal/b` treated as absolute | Match only at the start (after an optional `/`) | Open; RaftCLI avoids it by always sending `/<fs>/<path>` |
| Template BLE UUID `bb76…` differs from the library default `aa76…` | RaftCLI `app_config.rs:340` | Clients that try only `aa76…` can't find new apps | Template writes `aa76…` | Done (RaftCLI PR #2) |

## 9. Phases

Phase numbers here are for `raft fs`. Each phase leaves everything working.

| Phase | Work | Verify |
|-|-|-|
| **A** (done) | `DeviceConn` trait and `HttpConn`; `app_fs.rs` and `app_devtypes.rs` use `ctx.conn`; `DEVICE` parsing (HTTP only accepted); `-d`/`RAFT_DEVICE`; `cat` uses `download_file` | Existing behaviour over HTTP: `ls -r`, `get`, `put`, `sync --dry-run`, the default mirror, `devtypes push` |
| **B** (done) | `ricrest/` (HDLC, OverAscii, session, upload) and `SimulatedLink`; `RicrestConn` | Host tests: HDLC/OverAscii round-trips and fixed vectors; every `raft fs` command against the simulator, including a dropped block, a timer ack, device `ufCancel`, and a `filehash` mismatch |
| **C** (done) | `SerialLink`; `serial:`; WSL hand-off to `raft.exe` (§7.1) | A native-USB ESP32-S3 (Feather, a throwaway `raft new` app) from WSL: all commands, 200 KB each way, the FSImage mirror, devtypes, no reset on open (§7.2). Still to do: a USB-UART board, Linux, macOS |
| **D** (done) | `WebSocketLink` (`tungstenite` 0.30, blocking); `ws://` | The throwaway app's ESP32-S3 on WiFi: all commands, compared with HTTP (§9.4) |
| **E** (Windows done) | `BleLink` (btleplug 0.13 with the WinRT fix, CCCD write, `bledisconnect` on every exit), `ble` feature, `raft ble scan`, address cache, "may be held" hint | FeatherS3 and Axiom from Windows, macOS and Linux, on old and fixed firmware: scan, all commands, mirror; Ctrl-C mid-transfer leaves the device advertising |
| **F** (not planned) | `dfStart` download for speed on serial and WebSocket; tune block and batch sizes from measurements | Not needed: `filesection` downloads are as fast or faster on every link (WebSocket 52 KB/s against `dfStart`'s 35-37 KB/s; BLE 6-8 KB/s against about 5), and since RaftSysMods PR #9 they are read on the device's file system worker, off its main loop |

- B and C together are the first useful result: devices with no network can be managed.
- D is about a day once B is done.
- E is the largest phase, mostly platform testing.
- The Windows spike is done (§6.5). Before E, repeat it on macOS and Linux: characteristic access, which CCCD value
  `subscribe` writes, `mtu()`, and whether the link is released on disconnect.

**Test vectors** for B. They were recomputed with the spike's encoder, whose frames the device accepted. The first
one is exactly the first frame of every spike run, so it has been confirmed on a device. Confirm the others over
serial in phase C:

- `v`, msgNum 1, URL: `e7 01 02 00 76 00 f4 ca e7`
- `filelist`, msgNum 1: `e7 01 02 00 66 69 6c 65 6c 69 73 74 00 d1 dc e7`
- msgNum `0xE7` (escaping): `e7 d7 c7 02 00 76 00 02 43 e7`
- `crc16("123456789") = 0x29B1`

These include a trailing NUL after the URL, as raftjs sends it. The device doesn't need one.

### 9.1 Phase A in detail (done 2026-10-23)

Phase A changes no behaviour, apart from `cat` and the new ways to give the device. It moves every device call behind
`DeviceConn`, so phases B to E only add link types.

**Result:**
- All 98 tests pass, 9 of them new:
  - device spec parsing and rejection;
  - `open_device` (HTTP only; the port given twice);
  - resolving the device from the arguments, `-d` and the environment;
  - against `FakeConn`: sync with hash, delete and unchanged files; dry run and excludes; filesection chunking per
    link; device errors; `cat` and listing.
- Checked on the Axiom over HTTP (192.168.1.7), comparing with the installed raft 1.26.1:
  - identical output: `ls -r`, `ls --json`, `df`, `sync --dry-run --hash --delete`, the FSImage mirror
    `--dry-run`, and `get` of a 10 KB binary file;
  - new build only: `put`; `cat`, which is now binary-safe; missing-file errors (`Device error: nofile`); `rm`;
    `devtypes status` and `pull`; `devtypes push --dry-run`; the `-d` and `RAFT_DEVICE` forms.
  - raft 1.26.1 has no `devtypes`, so those commands were checked on the new build only.
- Device behaviour noticed: `filedelete` of a missing file replies `ok`, so `raft fs rm` reports "Deleted" for it.
  This was the same before.
- Differences from the plan below:
  - `wait_for_restart` returns `bool`, as the code it replaced did;
  - `rest` only parses, and `rest_ok` adds the `rslt` check. Callers that inspect a failure themselves (for
    example `devtypes status` looking for `failUnknownAPI`) use `rest`.

**The plan as written before the work:**

1. **`src/device_conn.rs`** (new):
   - the `DeviceConn` trait from §4.1, with `rest_raw`, `rest`, `put_file`, `describe`, `link_kind`;
   - `LinkKind` (`Http`, `WebSocket`, `Serial`, `Ble`), with `filesection_chunk()` and `default_timeout()` (§5.3);
   - `HttpConn { host, port }`, which wraps `http_client::http_get`, `http_get_timeout` and
     `http_post_multipart_file`;
   - `rest` parses the JSON and turns `"rslt":"fail"` into an error that keeps the device's `error` text, as
     `app_fs.rs` does now.
2. **`DeviceSpec` and `parse_device_spec(&str)`** (same file): the forms in §4.2.
   - In phase A only HTTP forms open. `ws://`, `serial:` and `ble:` parse, but fail with "not supported yet".
   - Unit tests cover every form, including a bare IP, `host:8080`, `http://host`, `serial:`, `serial:COM4@921600`,
     `ble:` and `ble:Axiom018`.
3. **`app_fs.rs`:**
   - `DeviceCtx { ip_addr, port, fs }` becomes `DeviceCtx { conn: Box<dyn DeviceConn>, fs }`;
   - replace the seven `http_get` calls (`fetch_listing`, `fs_list`, `download_file`, `fs_cat`, `delete_remote`,
     `fs_format`, `remote_hash`) with `ctx.conn.rest(...)`, and `upload_file`'s multipart POST with
     `ctx.conn.put_file(...)`;
   - messages that print `ctx.ip_addr` use `ctx.conn.describe()`;
   - `fs_cat` uses `download_file` (decision 2).
4. **`app_devtypes.rs`:**
   - `device_api` becomes `ctx.conn.rest(path)`;
   - `wait_for_device` becomes `DeviceConn::wait_for_restart(timeout)`. It is added to the trait in this phase:
     over HTTP it polls `v` as now, and RICREST links will reopen the link;
   - `device_ctx` builds the connection from `DeviceArgs`;
   - the pushed-CRC record is keyed by the device spec string (for HTTP, the same as `ip_addr` today).
5. **CLI arguments:** rename `IP_ADDRESS_OR_HOSTNAME` to `DEVICE` in `FsImageArgs`, `FsCommon` and the devtypes
   `DeviceArgs`.
   - The field can keep its name for a smaller diff, or become `device`.
   - Update the help text with the forms in §4.2.
   - Add `RAFT_DEVICE` as the default if decision 4 is yes (clap `env = "RAFT_DEVICE"`).
6. **Tests:**
   - `parse_device_spec` unit tests;
   - a `FakeConn` (an in-memory `DeviceConn`) so the sync planning in `fs_sync` can be tested without a device. It
     is the start of the phase B `SimulatedLink`, but answers at the `rest` level;
   - the existing `app_devtypes` tests still pass.
7. **Check on hardware:** the list in the phase A row, on the FeatherS3 or the Axiom over WiFi. The help text should
   show `DEVICE`.

Phase B can then start straight away. Its pieces (HDLC, OverAscii, frames, session, upload) are host-only, so
phase B doesn't wait for any hardware.

### 9.2 Phase B (done 2026-10-23)

- New code: `src/ricrest/` (§4.3), `src/link/` (§4.4), and `FakeConn::answer`, which is shared with the simulator.
- **120 tests pass**, 22 of them new (after the upload rule was revised on 2026-10-23):
  - HDLC: the plan's test vectors; round trips fed in pieces of 1, 7, 182 and 4096 bytes; corrupt frames dropped
    and counted;
  - OverAscii: every byte value; known encodings; text between and inside frames;
  - the wire formats; frame building and parsing;
  - against the simulator:
    - REST on every link kind;
    - upload block sizes per link (800, 800, 333), with `/<fs>/<path>` and the `filehash` check;
    - lost blocks with current and earlier firmware rules, mid-batch, at the start of a batch, and the last block,
      checking the blocks sent and the timer acks needed;
    - a device cancel, then a clean retry;
    - a silent device, which is given up on and cancelled, then a clean retry;
    - a damaged upload, which is detected and removed;
    - an empty file;
    - a lost reply (resent); duplicate replies (ignored); a corrupted reply (retried) on serial and BLE;
    - log text; message numbers wrapping past 255 without using 0;
  - every `raft fs` operation (`app_fs` functions) over RICREST on a simulated serial link with log text: sync
    with hash and delete, a second sync with nothing to do, get, cat, ls, df, put, rm, and `--fs sd`.
- The RICREST modules aren't used outside tests until phase C. `main.rs` allows dead code for them in non-test
  builds until then.
- **Left for phase C:**
  - `SerialLink` and `serial:` in `open_device`;
  - Ctrl-C sending `ufCancel` (no signal handling crate is in use yet);
  - a longer timeout for `reformatfs`;
  - `--show-log` and `-v` (bad-frame and stale-response counts);
  - the WSL hand-off;
  - checking the simulator's behaviour against a real device: block clamping, timer acks, `transferInProgress`
    and the duplicate-request rule.

### 9.3 Phase D (done 2026-10-23)

- `src/link/websocket.rs`:
  - tungstenite 0.30, blocking, on a `TcpStream` with a 5 s connect and write timeout;
  - an HDLC frame per binary message, at `ws://<host>[:port]<path>` (path default `/ws`);
  - as on serial, the socket is read only in `recv`, on the calling thread, and the device's pings are answered as
    they are read;
  - `reopen` connects again, looking the host name up again each time.
- `open_device`: `DeviceSpec::WebSocket`. Upload sizes stay at 800-byte blocks in batches of 10.
- **New test:** `RicrestConn` over a real WebSocket on localhost, against a server thread running the simulated
  device: a listing, a 20 KB upload, and a `filesection`. 125 tests in all.
- **`wait_for_restart` made robust**, after `devtypes push` over WebSocket failed:
  - it first waits for the device to go down (up to 10 s), as the link can still reach it before it restarts;
  - then it reopens the link and polls until it answers, opening again if the link drops while the device
    starts up.

  Before, an early reconnect reached the web server that was about to shut down, and the closed socket was never
  reopened.

### 9.4 Phase D results

Tested from WSL (WebSocket runs there natively; no hand-off). The device was the phase C throwaway app on the
ESP32-S3 Feather, joined to WiFi, with RaftSysMods PR #6.

| 200 KB | WebSocket | HTTP |
|-|-|-|
| Upload | 11.5 s, 17 KB/s (800-byte blocks, batches of 10; 20 was within noise, 4 and 40 were slower) | 9.6 s, 21 KB/s |
| Download | **3.9 s, 52 KB/s** (one connection, 4 KB `filesection` chunks) | 7.8 s, 26 KB/s (a connection per chunk) |

- **Also checked:** `ls`, `df`, `cat`, `rm`, `RAFT_DEVICE`, `sync --hash` (twice), the FSImage mirror dry run, and
  `devtypes status` and `push`. Nothing was dropped or resent.
- **The device's address changed after a restart.** It came back from `devtypes push` as 192.168.1.24 instead of
  .23 (DHCP), so a device named by IP address isn't found after a restart, over HTTP as over WebSocket.
  - Its mDNS name (`FsLinkTest59cc8a.local`) resolves from WSL and Windows.
  - With the name, `devtypes push` over WebSocket takes 18 s, as the device rejoins WiFi.
  - The README now suggests a hostname for commands that restart the device.
- Upload speed over WebSocket is set by the device. It caps blocks at 800 bytes (2/3 of the channel's 1200), and
  each block's flash write takes about 50 ms. HTTP's multipart upload writes bigger pieces.

### 9.6 Block size experiment (2026-10-23)

The question: do the device's file upload block limits (800 bytes on serial and WebSocket, 333 on BLE) slow down
fast devices?

**Where the limits come from:**
- `FileUploadOKTOProtocol` clamps a block to 2/3 of the channel's `inboundBlockLen`.
- WebSocket, serial, CommandSocket and MQTT register their channels with the default 1200 bytes, whatever the
  transport or memory; BLE uses `maxPktLen` (500).
- `ufStart` already negotiates the size (the client asks, the device's reply decides), so no protocol change is
  needed: the device's answer just has to reflect what it can take.

**Setup:**
- The throwaway app with PSRAM, built with three experimental libraries:
  - RaftWebServer: SysTypes `websockets[].inboundBlockLen` sets the WebSocket channel's limit (60000, so blocks of
    up to 40000). This is on the unpushed branch `experiment/ws-block-size`.
  - RaftCore with the MiniHDLC fix (below).
  - RaftSysMods #6.
- 200 KB uploads over WebSocket, with `RAFTCLI_UPLOAD_SIZES`.

**Found on the way:** MiniHDLC stored its frame limits in `uint16_t`, so on PSRAM devices `ProtocolRICSerial`'s
200000 became 3392. Every RICSerial frame over 3392 bytes was dropped, on any channel. This is fixed by RaftCore
PR #16 (`test_mini_hdlc`).

**Results:**

| Storage | 800-byte blocks | 4000 | 16000 | HTTP |
|-|-|-|-|-|
| LittleFS, Feather ESP32-S3 (two runs) | 17.6 KB/s | 19-20 | 18-19 | 21 |
| LittleFS, Axiom plant growth board (two runs) | 12 KB/s | 13-14 | 12 | 13-15 |
| SD card (FAT over SPI), plant growth board (median of 5) | 28 KB/s | 43 | **58** | - |

- **On LittleFS block size hardly matters.** The flash write is the bottleneck, and HTTP's multipart upload is no
  faster.
- **On an SD card, 16 KB blocks are about twice as fast as 800-byte blocks.** SD run times vary a lot between runs
  (FAT allocation), hence the medians.
- One 2000-byte run to SD failed after 60 s with a WebSocket send error; it wasn't repeated in 15 later runs.
- **Firmware updates** (`fileType: "fw"`) go through the same protocol and write raw flash, so they are likely to
  gain the same way (not measured).

**Firmware change made (RaftWebServer PR #6, needs RaftCore PR #16):**
- With PSRAM, the WebSocket channel's `inboundBlockLen` defaults to 24000, capped at the websocket's
  `pktMaxBytes`, so blocks can be up to 16000. Without PSRAM it is unchanged. SysTypes
  `websockets[].inboundBlockLen` overrides it.
- RaftCLI asks for 16000-byte blocks on WebSocket, and older firmware clamps them to 800. raftjs asks for 500
  unless a caller sets otherwise.
- Measured on the plant growth board with the defaults: SD 55 KB/s (median of 5) against 29 KB/s for 800-byte
  blocks; LittleFS 12 KB/s either way.
- Not changed:
  - The 2/3 factor in `FileUploadOKTOProtocol`: changing it would raise the serial and BLE limits too, and those
    aren't measured.
  - Serial: the receive buffer limits it (§7.2).
  - BLE: limited by the MTU and connection interval; measure in phase E.
  - CommandSocket and MQTT: not tested.
- Memory safety, after review:
  - RaftCore PR #17 limits block and batch sizes by the largest free block.
  - RaftWebServer PR #6 limits received WebSocket messages to `pktMaxBytes` and checks memory before a receive
    buffer grows. It also fixes fragmented WebSocket messages and closes the link (1009) on a frame header over
    the limit.
  - With these, 16000-byte blocks to SD: 58-66 KB/s.
- **4000-byte blocks (2026-10-24):** the device writes each block in its main loop, so bigger blocks mean longer
  pauses there. RaftCLI and raftjs (PR #20) ask for 4000 on WebSocket: 52 KB/s to SD, against 57 for 16000 and 29
  for 800. Taking the writes off the main loop is planned in RaftCore's `devdocs/flash-write-blocking-plan.md`
  (measurements in RaftCore PR #19).

### 9.5 Phase E (Windows done 2026-10-24)

As built:

- **btleplug** 0.13.5 with the WinRT fix (§6.5, finding 1): `GattDeviceService::OpenAsync(SharedReadAndWrite)` before
  characteristic discovery, in `get_characteristics`. It is on the fork `robdobsn/btleplug`, branch
  `fix/winrtble-open-gatt-service` (one commit on upstream master, which is the 0.13.5 release), used through
  `[patch.crates-io]` by git, pinned to the commit. **Still to do:** the pull request upstream; once it is in a
  release, drop the patch.
- **`src/link/ble.rs`** (`BleLink`, cargo feature `ble`, on by default):
  - a thread runs a single-threaded tokio runtime and owns the connection. `send` passes HDLC-encoded frames to it
    over a channel and waits for the writes; notifications are decoded into frames on another channel for `recv`;
  - the scan is unfiltered and matches a device by the Raft service UUID in any packet from an address and the
    name from that address (§6.2). `ble:<x>` matches an exact name or address first, then a unique name prefix
    (scanning another 1.5 s for an exact match); `ble:` takes the only Raft device seen in 5 s. A named scan stops
    as soon as the device is seen, or after 10 s;
  - after subscribing it writes notify (`0x0001`) to the CCCD, and sizes writes from `mtu()` (509 bytes on Windows);
  - **setup retries:** straight after a previous connection is released, Windows can still have the old link up:
    `connect` returns at once and the service has no characteristics. 4 of 10 back-to-back runs failed this way.
    Connect and discovery are now tried up to 3 times, 1.5 s apart: 10 of 10;
  - **release:** `bledisconnect` (msgNum 255, its reply taken by the link) up to 3 times, 2 s each, when the link
    closes (drop) and from a Ctrl-C handler (`ctrlc` crate) for every open link. A warning says how to release the
    device if it doesn't confirm;
  - `reopen` (after a restart) connects again by the name it had.
- **Address cache** in `<cache dir>/raftcli/ble_devices.json` (name to address), written after each connection. On
  Windows a named device (or a prefix of exactly one cached name) is connected to with `add_peripheral` and no scan,
  falling back to a scan after 5 s. macOS and Linux always scan (btleplug can't reach an unseen device there).
- **`raft ble scan [-t secs]`** lists name, address, RSSI and service. In WSL it runs in `raft.exe`, as `ble:`
  device commands do.
- The not-found error lists the Raft devices that were seen and says the device may be held by another connection,
  and how to release it (`bledisconnect` in `raft monitor`, `/api/bledisconnect` over WiFi, or a restart).
- `--ble-service`, `--ble-cmd`, `--ble-resp` and `--ble-mtu` on every device command.
- Linux builds need `libdbus-1-dev` (added to the CI and release workflows); `--no-default-features` builds without
  BLE.

**Results** (Windows 11 from WSL through `raft.exe`; Axiom on firmware built with RaftSysMods `main`, i.e. with the
§8 BLE fixes; plant growth board, same firmware generation; 2026-10-24):

| Test | Result |
|-|-|
| `raft ble scan` | Both devices, about 7 s for a 6 s scan |
| `raft fs ls` / `df`, first connection (scan) | 7 s in all |
| Same, cached address | 2-4 s in all, 10 of 10 (after the setup retry) |
| `put` 200 KB | 19.4 s in all (about 12 KB/s with the connection); 15-17 KB/s once transferring; 0 bad frames, 0 resends |
| `put` 1 MB | 65 s, 16.1 KB/s |
| `get` 200 KB | 30.2 s, identical |
| `cat`, `rm`, `ls -r`, `sync` (dry run, upload, `--hash` unchanged), FSImage mirror `--dry-run` | All correct |
| `ble:`, two devices in range | Error listing both (with a 3 s scan it once took the only one seen yet: now 5 s) |
| `ble:Marty` (absent), `ble:Axiom` (two match) | Errors listing the devices seen / matched, with the release hint |
| WSL hand-off (`raft fs ... ble:`, `raft ble scan`) | Works |
| Plant growth board (`ble:AxiomPlant`) | Works |
| Ctrl-C at 19% of a 1 MB upload (Windows Terminal, by hand) | "Releasing the BLE device...", exit; the Axiom advertised again at once |

**Not yet verified:**

- **Firmware without the §8 fixes** (the CCCD write is meant to cover it).
- **macOS and Linux** (BlueZ may refuse the CCCD write, which is ignored), and the FeatherS3.

## 10. Decisions

All decided on 2026-10-23: each as recommended, with decision 4 done as `-d/--device` plus `RAFT_DEVICE` (§4.2).

1. **(Decided: `aa76…`, done.)** **The BLE default service UUID for new apps.** `aa76…`, the library and raftjs
   default, or keep `bb76…`? The CLI scans for both either way. Recommendation: change the template to `aa76…`
   unless `bb76…` was chosen on purpose to keep new apps apart from older devices.
2. **(Decided: switch, done.)** **`cat` over HTTP.** Switching to `filesection` makes it binary-safe and the same
   on every link, but slightly slower for large text files. Recommendation: switch.
3. **The device-side fixes in §8.** Done: the RaftSysMods ones are on `main` (`0753ed0`) and the RaftCore upload fix
   is merged (PR #14). The other RaftCore items are still open and don't block any phase.
4. **(Decided, done.)** **`RAFT_DEVICE` default.** Done as a default for the new `-d/--device` option. A plain
   environment default for the positional `DEVICE` would be ambiguous: in `raft fs get config.json`, clap would take
   `config.json` as the device (§4.2).
5. **(Decided: pull request and fork, for phase E.)** **The btleplug WinRT fix (§6.5, finding 1).** Options:
   - a pull request upstream, with RaftCLI using a git fork through `[patch.crates-io]` until it is released;
   - vendor the crate;
   - write the Windows link directly on the `windows` crate.
   Recommendation: the pull request and fork. The change is a few lines and the rest of btleplug works.
   `bluest` (the other maintained cross-platform crate) was tried. It writes notify, not indicate, which avoids
   finding 2, and it reports the write size. But it has the same access and link-release problems, and on Windows
   its `disconnect` does nothing. Switching to it wouldn't remove the workarounds.
6. **Firmware CCCD fix (§8).** Done (RaftSysMods `0753ed0`), and tested on the Axiom. RaftCLI keeps writing notify to the CCCD
   because it works on old and new firmware.

## 11. Not in scope

- File commands inside `raft monitor` (native-transports plan, phase 3).
- `raft ota` over RICREST (native-transports plan, phase 6). The upload code here is reused for it with
  `fileType: "fw"`.
- Bridged devices (`bridgeserialN`).
