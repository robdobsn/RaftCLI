# Filesystem Management — Design & Implementation Plan

> Status: **Draft / planning**. This document plans a new `raft fs` command group for the
> RaftCLI that lets a developer manage the filesystem of a running Raft-firmware device
> over its REST API, in the same spirit as the existing `raft ota` command.

## 1. Motivation

A Raft device built on the Raft framework runs the [`FileManager`](../../RaftSysMods/components/FileManager/FileManager.cpp)
SysMod, which exposes a small REST API for working with the on-device filesystem
(LittleFS/SPIFFS in flash, and optionally an SD card). Today the only ways to use that API
are a browser, hand-written `curl` commands, or the device web UI. There is no first-class
CLI workflow.

The goal is to add a `raft fs` sub-command group to RaftCLI so that, given a device IP
address or hostname, a developer can:

- **list** files and folders on the device filesystem (with sizes and disk usage),
- **download / get** a file from the device to the host,
- **upload / put** a file from the host to the device,
- **delete** a file on the device,
- **show** (cat) the contents of a text file,
- optionally **reformat** the filesystem,
- and **sync / mirror** a host folder to the device.

The **primary motivating use case** is updating the device's web UI: a build produces a set of
web files (HTML/JS/CSS/assets) that live on the device filesystem, and the developer wants to
push the whole set to a running device in one command — the on-device equivalent of an OTA
update, but for the filesystem rather than the firmware image. This must:

- upload the new/changed files, and
- **delete files that no longer exist** in the source set, so stale assets don't linger.

Because some devices have very little free flash, the sync must be able to **delete obsolete
files first to free space before uploading** the new ones (see §4.8). This makes `raft fs sync`
a first-class feature of this work, not an afterthought.

This mirrors the existing OTA flow ([`app_ota.rs`](../src/app_ota.rs)), which already performs
an authenticated-by-network REST upload (`POST /api/espFwUpdate`) with progress reporting,
so the architecture and much of the transport code can be reused.

## 2. Background — how the device side works

### 2.1 FileManager REST endpoints

The `FileManager` SysMod ([`RaftSysMods/components/FileManager/FileManager.cpp`](../../RaftSysMods/components/FileManager/FileManager.cpp))
registers the following endpoints. All are reached under the web server's `/api/` prefix
(the registered name is the first path segment, e.g. `filelist` → `GET /api/filelist/...`).

| Endpoint | Method | Purpose |
|----------|--------|---------|
| `filelist`   | GET  | List files in a folder on a filesystem |
| `fileread`   | GET  | Read a file's contents (returned as `text/plain`) |
| `filedelete` | GET  | Delete a file |
| `fileupload` | POST | Upload a file (streamed, multipart/form-data) |
| `reformatfs` | GET  | Reformat a filesystem (causes a device restart) |

Important conventions (verified in the source):

- **Filesystem selector** is the first path argument: `local` (flash LittleFS/SPIFFS — also
  accepts the alias `spiffs`) or `sd` (SD card). It may be **blank to mean the default**
  filesystem.
- **`/` inside folder/file paths must be replaced by `~`** in the request URL. The device
  reverses this with `folderStr.replace("~", "/")`. (See `apiFileList`, `apiFileRead`,
  `apiDeleteFile`.)
- Request strings are parsed positionally with
  `RestAPIEndpointManager::getNthArgStr(reqStr, n)` — i.e. simple slash-delimited path
  segments, not query strings.

#### `filelist` — list files

```
GET /api/filelist/<fs>/<folder-with-~-for-slash>
```

Returns JSON produced by `FileSystem::formatJSONFileInfo`
([`RaftCore/components/core/FileSystem/FileSystem.cpp`](../../RaftCore/components/core/FileSystem/FileSystem.cpp)):

```json
{
  "req": "filelist/local/",
  "rslt": "ok",
  "fsName": "local",
  "fsBase": "/local",
  "diskSize": 1048576,
  "diskUsed": 20480,
  "folder": "/local/",
  "files": [
    { "name": "config.json", "size": 223, "isDir": 0 },
    { "name": "index.html",  "size": 1840, "isDir": 0 }
  ]
}
```

Error responses use `{"rslt":"fail",...}` style (e.g. `"unknownfs ..."`, `"nofs"`,
`"nofolder"`, `"fsbusy"`).

Notes / limitations discovered in the source:
- The listing is **a single directory level** (it `opendir`/`readdir`s one folder; it does not
  recurse). Sub-folder traversal must be done by the client issuing further `filelist` calls.
- There is **no explicit `type`/`isDir` field** per entry — only `name` and `size`. Detecting
  folders from the list alone is not currently possible (a directory and a zero-byte file look
  similar). This is a gap worth confirming/raising (see §8 Open Questions).
- When `CacheFileSysInfo` is enabled the root listing may come from a cache.

#### `fileread` — read a file

```
GET /api/fileread/<fs>/<filename-with-~-for-slash>
```

Returns the **whole file** with content type `text/plain`. On the device this calls
`FileSystem::getFileContents`, which returns a NUL-terminated C string. **This is binary-unsafe**:
a file containing a `0x00` byte will be truncated, and the response is not length-delimited at the
API layer. For text/config files this is fine; for arbitrary binary downloads it is not reliable.
See §8 for the recommended download strategy.

#### `filedelete` — delete a file

```
GET /api/filedelete/<fs>/<filename-with-~-for-slash>
```

Returns `{"rslt":"ok"}` or `{"rslt":"fail"}`.

#### `fileupload` — upload a file (streamed)

```
POST /api/fileupload          (Content-Type: multipart/form-data)
```

This is the same transport mechanism the OTA endpoint uses. On the device,
`FileManager::apiUploadFileBlock` forwards each received block to
`ProtocolExchange::handleFileUploadBlock(...)`
([`RaftCore/components/comms/ProtocolExchange/ProtocolExchange.cpp`](../../RaftCore/components/comms/ProtocolExchange/ProtocolExchange.cpp)),
which:
1. on the first block, opens a `FileStreamSession` of type
   `FILE_STREAM_CONTENT_TYPE_FILE` with flow type `FILE_STREAM_FLOW_TYPE_HTTP_UPLOAD`,
2. writes each subsequent block to that session, and
3. the `FileManager::apiUploadFileComplete` callback returns `{"rslt":"ok"}` when the POST
   completes.

The upload is handled by `FileUploadHTTPProtocol`
([`RaftCore/components/comms/FileStreamProtocols/FileUploadHTTPProtocol.cpp`](../../RaftCore/components/comms/FileStreamProtocols/FileUploadHTTPProtocol.cpp)) —
the file content travels in the HTTP POST body (multipart), not in RICREST data frames.

**Target filename / path**: the destination filename is carried in `FileStreamBlock.filename`,
which the web server populates from the request. The OTA path uses the multipart
`Content-Disposition: ... filename="<name>"`. The exact rule for how the *target path*
(filesystem + sub-folder) is chosen for `fileupload` is implemented in the RaftWebServer
component, **which is not part of this workspace** and therefore must be confirmed against a
real device (see §8 Open Questions). The likely shape is either
`POST /api/fileupload/<fs>/<filename>` (path carries the destination) and/or the multipart
`filename` field.

### 2.2 The on-device FileSystem core

`FileSystem` ([`RaftCore/components/core/FileSystem/FileSystem.h`](../../RaftCore/components/core/FileSystem/FileSystem.h))
is the singleton (`extern FileSystem fileSystem;`) that backs the FileManager. Relevant
capabilities that exist in core but are **not all exposed over REST today**:

- `getFilesJSON(...)` → backs `filelist`.
- `getFileContents(...)` → backs `fileread` (text, whole-file).
- `setFileContents(...)` → write a whole file from a string (not directly exposed as its own
  endpoint; uploads go through the streaming session instead).
- `deleteFile(...)` → backs `filedelete`.
- `getFileInfo(...)` / `exists(...)` / `pathType(...)` → existence + dir/file stat.
- `getFileSection(fs, filename, start, len, ...)` → **binary-safe ranged read**. Not exposed
  over REST currently, but is the natural primitive for a robust binary download.
- `getFileSystemSize(...)` → total/used bytes (also surfaced in the `filelist` JSON).
- `reformat(...)` → backs `reformatfs`.

Filesystem names/paths (constants in `FileSystem.h`):
- local: name `local` (alias `spiffs`), base path `/local`, partition label `fs`.
- sd: name `sd`, base path `/sd`.

### 2.3 ESP-IDF filesystem context

The underlying storage is one of:
- **LittleFS** (default; `LocalFsDefault` config, value `littlefs`), or
- **SPIFFS** (`spiffs`), or
- **SD card** over SPI (pins configured via `SDMOSI/SDMISO/SDCLK/SDCS`, enabled by `SDEnabled`).

Consequences relevant to the CLI:
- **SPIFFS has no real directories** — "folders" are an illusion created by `/` in flat
  filenames. LittleFS does support directories. The CLI should not assume mkdir/rmdir
  semantics for `local` and should treat folders as path prefixes.
- **No mkdir / rmdir / rename endpoints** exist today. Creating a "folder" happens implicitly by
  uploading a file with a `/`-containing name (LittleFS). The CLI plan should not promise
  mkdir/rename until a device endpoint exists.
- The flash filesystem is small (hundreds of KB to a few MB). Uploads should chunk and report
  progress, and the CLI should surface `diskSize`/`diskUsed` so users can see headroom.
- Reformatting **restarts the device** (`SysManager::systemRestart()` is called when
  `reformat` returns "restart required").

### 2.4 How the existing OTA command talks to the device (reuse target)

[`app_ota.rs`](../src/app_ota.rs) already contains everything needed for an HTTP upload with
progress, with **no extra crate dependencies** (it builds the HTTP request by hand on a
`std::net::TcpStream`):

- `DataRateTracker` — rolling KB/s rate over a 5 s window.
- `ProgressTracker` / `ProgressReader` — chunked reader that reports % and rate every 500 ms.
- `perform_ota_flash_basic_http_with_streaming(...)` — opens a TCP socket, writes a
  `multipart/form-data` POST with a hand-built boundary, streams the file, reads the response,
  and checks for `200 OK` + `"rslt":"ok"`.
- A `curl` fallback path (`--use-curl`).

The new `fs upload` command can reuse this almost verbatim (different URL, same machinery).
The `fs list`/`get`/`delete` commands need a simple **HTTP GET** helper, which does not exist
yet in the codebase.

### 2.5 Relevant CLI plumbing already available

From [`raft_cli_utils.rs`](../src/raft_cli_utils.rs) and [`main.rs`](../src/main.rs):
- Sub-commands are defined as a `clap` `enum Action` and dispatched in `main()`.
- `utils_get_sys_type(...)` resolves the system type from the app folder (used by OTA to build
  the firmware image path — not strictly needed for fs ops, but available).
- `read_build_info` / `write_build_info` persist last-used settings (e.g. `last_port`) into
  `raft.info`. We can extend this to remember the **last device IP/hostname** so `raft fs`
  commands don't always need it re-typed.

## 3. Proposed CLI surface

Add a new top-level command group, consistent with existing naming (`ota`, `ports`, `libs`):

```
raft fs <subcommand> [options]
```

Alias: `raft fs` (and short alias e.g. `raft f:` is not needed — keep it explicit).

Common options (applicable to all subcommands, resolved like OTA's `ip_addr`/`ip_port`):

| Option | Description |
|--------|-------------|
| `<ip_addr>` (positional) | Device IP address or hostname (remembered in `raft.info` if omitted) |
| `-p, --port <port>` | HTTP port (default `80`) |
| `--fs <local\|sd>` | Target filesystem (default: device default / `local`) |
| `--use-curl` | Use `curl` for transfers (parity with OTA), else native sockets |
| `-a, --app-folder <path>` | App folder, only used for `raft.info` persistence |

### 3.1 Subcommands

| Command | Maps to | Notes |
|---------|---------|-------|
| `raft fs list [folder]` | `GET /api/filelist/<fs>/<folder>` | Pretty table: name, size, plus a header line with disk size/used. `--json` for raw. `--recursive` issues nested `filelist` calls (client-side recursion). |
| `raft fs get <remote> [local]` | download | See §4.4 for strategy. Defaults `local` to basename of `<remote>` in CWD. `--out -` to stream to stdout. |
| `raft fs put <local> [remote]` | `POST /api/fileupload` | Reuses OTA streaming uploader. Defaults `<remote>` to basename of `<local>`. Progress + rate. |
| `raft fs delete <remote>` | `GET /api/filedelete/<fs>/<remote>` | `-y/--yes` to skip confirmation. |
| `raft fs cat <remote>` | `GET /api/fileread/<fs>/<remote>` | Text only; warns if binary suspected. |
| `raft fs df` (or `info`) | `GET /api/filelist/<fs>/` | Show disk size/used/free for the filesystem. |
| `raft fs format` | `GET /api/reformatfs/<fs>[/force]` | **Destructive**: require explicit `--yes`; warn that the device will reboot. |
| `raft fs sync <localdir> [remotedir]` | list + diff + delete + put | Mirror a host folder onto the device (primary web-UI use case). Deletes stale remote files, then uploads. See §4.8. |

### 3.2 Examples

```
raft fs list 192.168.1.42
raft fs list 192.168.1.42 web --recursive
raft fs put 192.168.1.42 ./dist/index.html index.html
raft fs get 192.168.1.42 config.json
raft fs cat 192.168.1.42 config.json
raft fs delete 192.168.1.42 oldlog.txt -y
raft fs df 192.168.1.42 --fs sd
raft fs sync 192.168.1.42 ./build/webui --delete
```

## 4. Implementation design (Rust)

### 4.1 New module: `src/app_fs.rs`

Add `mod app_fs;` to [`main.rs`](../src/main.rs) and a new `Action::Fs(FsCmd)` variant.
`FsCmd` is a `clap` struct with a nested `#[clap(subcommand)]` enum `FsSub` for
`List/Get/Put/Delete/Cat/Df/Format/Sync`.

`app_fs.rs` responsibilities:
- Argument resolution (ip/port/fs, with `raft.info` persistence of the last device).
- URL construction with the `~`-for-`/` encoding helper.
- HTTP GET helper (new) and HTTP POST upload (reuse OTA code).
- JSON response parsing with `serde_json` (already a dependency).
- Pretty output / tables and `--json` passthrough.

### 4.2 Path encoding helper

```rust
/// Encode a device-relative path for the FileManager REST API:
///  - strip any leading '/'
///  - replace '/' with '~'
fn encode_device_path(path: &str) -> String {
    path.trim_start_matches('/').replace('/', "~")
}
```

The filesystem selector and the encoded path are joined as
`/<endpoint>/<fs>/<encoded-path>`. A blank `fs` (default) should be handled carefully because
the device expects the fs as arg index 1; sending an empty segment (`//`) needs verification —
safer to default to `local` explicitly unless `--fs` says otherwise (confirm in §8).

### 4.3 HTTP GET helper

There is **no HTTP client in the project today** (the OTA code hand-rolls HTTP over
`TcpStream`). Two options:

- **Option A (preferred for consistency, zero new deps):** add a tiny `http_get` helper in
  `app_fs.rs` that opens a `TcpStream`, writes a `GET ... HTTP/1.1\r\nHost:..\r\nConnection: close`
  request, reads the full response, splits headers/body on `\r\n\r\n`, and returns
  `(status_code, headers, body_bytes)`. This matches the existing style in `app_ota.rs`.
- **Option B:** add a small HTTP client crate. `ureq` (pure-Rust, blocking, no async runtime)
  is the lightest reasonable choice and would also simplify multipart uploads and binary
  downloads with proper `Content-Length` handling. **However**, the project explicitly favours
  minimal dependencies (see the serial-monitor V2 design doc), so **Option A is the default**
  unless binary download robustness (§4.4) pushes us to a real client.

A `curl` fallback (`--use-curl`) should be provided for all operations, matching OTA.

### 4.4 Download strategy (the tricky part)

`fileread` is text-only and binary-unsafe (§2.1). Because the firmware work (Phase 0, §7.1)
adds a binary-safe ranged read endpoint, the CLI's preferred path is straightforward:

1. **Primary: ranged binary read** via the Phase 0 endpoint
   `GET /api/filesection/<fs>/<path>/<start>/<len>`. To remain binary-safe across **all** Raft
   transports (HTTP, BLE, serial — the API is transport-agnostic and responses are returned as a
   text `String`), the section bytes are **base64-encoded inside a JSON response** rather than
   sent as raw `application/octet-stream`. Shape:
   `{"rslt":"ok","fs":..,"fname":..,"start":..,"len":..,"total":..,"data":"<base64>"}`.
   The CLI base64-decodes `data` and uses `start`/`len`/`total` to page through large files.
   Backed by `FileSystem::getFileSection`. **Implemented.**
2. **Fallback (older firmware): `GET /api/fileread/...`** for text files, with a warning that
   binary content may be truncated.
3. **Fallback (if present): direct static GET** of the file path, where the web server serves
   filesystem files directly with a correct `Content-Length`. Used only if available on the
   target firmware (confirm prefix — §8.3).

`raft fs cat` always uses `fileread` (text) and is explicit about its text-only nature.

### 4.8 Sync (mirror a host folder → device)

`raft fs sync <localdir> [remotedir]` mirrors a local directory (typically a built web UI) onto
the device filesystem. This is the headline feature, so it is designed carefully around the
constraint that **flash space may be very limited**.

Algorithm:

1. **Enumerate local files** under `<localdir>` (recursively), recording relative path + size
   (+ optionally a content hash; see below).
2. **Enumerate remote files** under `[remotedir]` via `filelist` (client-side recursion; see
   the folder-detection caveat in §5). Record name + size.
3. **Compute the diff** into three sets:
   - **to-delete**: remote files not present locally (stale assets),
   - **to-upload**: local files missing remotely or whose size/hash differs,
   - **unchanged**: present on both with matching size/hash (skipped).
4. **Delete first, then upload** (the key ordering): issue `filedelete` for every to-delete
   file *before* uploading, so obsolete files free their space before new content is written.
   This is what allows a near-full device to be updated without running out of room.
5. **Upload** each to-upload file via the streaming `fileupload` POST (§4.5), with per-file and
   overall progress.
6. **Report** a summary: N deleted, N uploaded, N unchanged, bytes transferred, final
   `diskUsed`/`diskSize`.

Options:

| Option | Behaviour |
|--------|-----------|
| `--delete` | Enable deletion of stale remote files (mirror mode). Without it, sync only adds/updates (safe additive mode). |
| `--delete-first` (default on when `--delete`) | Perform deletions before uploads to free space. Could be made opt-out for speed on roomy devices. |
| `--dry-run` | Show the planned delete/upload/skip sets without changing the device. |
| `--prune-empty` | Best-effort removal of now-empty remote folders (LittleFS only; depends on device support — see §8). |
| `-y, --yes` | Skip the confirmation prompt (deletions are destructive). |

Change detection:

- **Primary: content hash/CRC.** With the Phase 0 firmware (§7.1, F2), `filelist` (or a
  dedicated endpoint) reports a per-file CRC/hash, so sync skips unchanged files reliably
  without downloading them. This is the intended default once the reference firmware is built.
- **Fallback: size only.** Against older firmware that only reports `size`, fall back to
  size-based comparison (cheap, but won't catch same-size edits) with a note to the user.

Space-safety details:

- If the device reports insufficient free space even after deletions, sync should **stop with a
  clear error** rather than leaving the filesystem half-updated, and report which files were and
  weren't written.
- Consider uploading a **temp name then no rename is available** (there is no rename endpoint
  today — §8), so atomic per-file replace isn't currently possible; document that an interrupted
  sync can leave a partially-written file. A future device-side rename endpoint would enable
  write-temp-then-rename atomicity.

### 4.5 Upload implementation

Refactor the reusable parts of [`app_ota.rs`](../src/app_ota.rs) (`DataRateTracker`,
`ProgressTracker`, `ProgressReader`, and the streaming multipart POST) into a shared module —
e.g. `src/http_upload.rs` — and have both OTA and `fs put` call it with different URLs:

- OTA: `POST /api/espFwUpdate`, multipart filename `<systype>.bin`.
- fs put: `POST /api/fileupload[/<fs>/<remote>]`, multipart filename `<remote>`.

Keep the existing OTA public function signature stable; just delegate its body to the shared
helper to avoid regressions.

The uploader must:
- set the multipart `Content-Disposition` filename to the **target device filename**,
- compute `Content-Length` correctly (it already does),
- stream in chunks with progress, and
- verify the response contains `200 OK` and `"rslt":"ok"`.

### 4.6 Response parsing & exit codes

- Parse JSON responses with `serde_json::Value`; treat `rslt != "ok"` as an error and print the
  device's error string.
- Map failures to non-zero process exit codes (consistent with how `main.rs` exits `1` on OTA
  failure).
- Network/timeout errors should produce a clear message including the device address and port.

### 4.7 Output formatting

- `fs list` default: a compact table
  ```
  Filesystem: local   Size: 1024.0 KB   Used: 20.0 KB   Free: 1004.0 KB
  SIZE      NAME
  223       config.json
  1840      index.html
  ```
- `--json` prints the raw device JSON.
- Sizes shown human-readable (reuse a small formatter; no new crate needed).

## 5. Edge cases & safety

- **Destructive ops** (`delete`, `format`) require confirmation unless `--yes`. `format` also
  warns about the device reboot.
- **`~` collisions**: filenames legitimately containing `~` would clash with the path encoding.
  This is a pre-existing device-API constraint; document it and avoid double-encoding.
- **Folder detection**: because `filelist` has no `isDir` flag, `--recursive` recursion must be
  heuristic (try `filelist` on each entry; treat success as a folder). Flag this clearly and
  consider proposing a device-side `isDir`/`type` field (§8).
- **SPIFFS vs LittleFS**: don't expose mkdir/rmdir; folders are path prefixes.
- **Large files vs small flash**: check `diskUsed`/`diskSize` headroom before upload when known;
  surface device "out of space"/failure responses.
- **WSL parity**: unlike serial/flash commands, `fs` talks over the network (TCP), so the WSL
  → Windows `raft.exe` delegation used for USB serial is **not required** here. Confirm no
  WSL-specific networking issues.
- **Auth**: the REST API is currently unauthenticated (LAN-trusted). Note this; don't add
  credentials handling unless the firmware grows it.

## 6. Testing plan

- **Unit**: `encode_device_path`, JSON parsing of representative `filelist`/`filedelete`
  responses, multipart header construction, human-size formatting.
- **Integration (mock)**: a tiny local HTTP server fixture (or recorded responses) to exercise
  list/get/put/delete without hardware.
- **On-device**: manual matrix against a real Raft device — list root + sub-folder, upload a
  text file and a binary file, download both back and byte-compare, delete, df, and (carefully)
  format on a scratch device. Test both `local` and `sd` where available.
- **curl parity**: verify `--use-curl` produces identical results for each op.

## 7. Phased delivery

The firmware (RaftCore + RaftSysMods) work is sequenced **first**, so that the CLI is built and
tested against a device whose API already has the binary-safe, sync-friendly capabilities it
needs. Doing the firmware first avoids building the CLI against today's gaps (text-only reads,
no folder/`isDir` info, no content hash) and then having to rework it.

### Phase 0 — Firmware target (RaftCore + RaftSysMods) — **done**

Goal: produce a reference firmware whose FileManager/FileSystem API fully supports robust
listing, binary-safe download, and content-hash-based sync. See §7.1 for the detailed device
change list. Status summary:

1. **`filelist` enrichment** — added per-entry `isDir` to the JSON (both the immediate and the
   ESP32 cached code paths), so the CLI can recurse folders reliably. **Done (F1).**
2. **Binary-safe ranged download** — added `GET /api/filesection/<fs>/<path>/<start>/<len>`
   returning base64 data in JSON (transport-safe), wrapping `FileSystem::getFileSection`.
   **Done (F3).**
3. **Per-file content hash** — added `GET /api/filehash/<fs>/<path>` returning a CRC16-CCITT of
   the file (computed in bounded chunks), for content-based sync diffing. **Done (F2).**
4. **Free-space query** — satisfied by the existing `filelist` `diskSize`/`diskUsed` fields; no
   new endpoint required. **Done (F5).**
5. **Confirm/define `fileupload` addressing** — still to verify against the (out-of-workspace)
   RaftWebServer. **Open (F4).**
6. *(Optional / nice-to-have)* **rename** and **bulk delete** endpoints — not yet implemented
   (F6/F7).

Deliverable: a built reference systype (firmware) exposing the above, used as the test target
for all CLI phases below.

### Phase 1 — Core CLI read/transfer (against the Phase 0 firmware) — IMPLEMENTED
Implemented in `src/app_fs.rs` (commands) + `src/http_client.rs` (dependency-free HTTP
GET / multipart POST), wired into `src/main.rs` as `raft fs`.
- `raft fs list` (+ `--json`, `--recursive`, `df`), using the enriched `filelist` (`isDir`).
- `raft fs put` (streaming multipart upload to `fileupload`, remote name in the multipart
  filename field).
- `raft fs delete` (`rm`), `raft fs cat`.
- `raft fs get` using the binary-safe ranged `filesection` endpoint (base64 paging via
  `start`/`len`/`total`).
- `raft fs sync` (size diff by default, `--hash` for CRC16-CCITT content diff, `--delete`
  with delete-first ordering, `--dry-run`) — the primary web-UI update workflow.
- `raft.info` persistence of last device address (`last_ip_addr`).

> Open item carried from Phase 0 F4: the exact `fileupload` destination addressing
> (filesystem selector + sub-folder path) needs live-device verification; the CLI currently
> sends the remote path in the multipart `filename` field against the default filesystem.

### Phase 2 — Robustness & compatibility
- `--recursive` listing and recursive `get` (relies on Phase 0 `isDir`).
- Graceful **fallback path for older firmware** that lacks the Phase 0 additions (size-only
  diff, `fileread` download with binary warning), so the CLI still does something useful against
  devices that haven't been updated.
- Resumable/ranged download using the Phase 0 `filesection` endpoint.

### Phase 3 — Convenience
- `sync` polish: `--prune-empty`, atomic per-file replace (uses the Phase 0 rename endpoint),
  parallel uploads, retries.
- `raft fs format` polish, progress UX improvements.

### 7.1 Firmware (RaftCore / RaftSysMods) change list — Phase 0 detail

| # | Change | Where | Notes |
|---|--------|-------|-------|
| F1 ✅ | Add `isDir` to each entry in `filelist` JSON | `FileSystem::fileInfoGenImmediate` + the ESP32 cache path (`CachedFileInfo`, `fileInfoCacheToJSON`, `fileSystemCacheService`) ([`RaftCore/components/core/FileSystem/FileSystem.cpp`](../../RaftCore/components/core/FileSystem/FileSystem.cpp)) | Uses `S_ISDIR(st.st_mode)`. Enables reliable client recursion. |
| F2 ✅ | Add `GET /api/filehash/<fs>/<path>` returning CRC16-CCITT | new `FileManager::apiFileHash` ([`RaftSysMods/components/FileManager/FileManager.cpp`](../../RaftSysMods/components/FileManager/FileManager.cpp)) | Reuses `MiniHDLC::crcUpdateCCITT` (same CRC the file-stream protocols use). Reads the file in 2 KB chunks so RAM use is bounded. On-demand (CLI calls only for size-matched files), avoiding per-list hashing cost. |
| F3 ✅ | New binary-safe ranged read `GET /api/filesection/<fs>/<path>/<start>/<len>` returning base64 in JSON | new `FileManager::apiFileSection` wrapping `FileSystem::getFileSection` | Removes the NUL-truncation limitation of `fileread`; base64-in-JSON keeps it safe over BLE/serial/HTTP. Enables robust/resumable `get`. |
| F4 | Confirm & document `fileupload` destination addressing; ensure sub-folder targets work on LittleFS | `FileManager::apiUploadFileBlock` / RaftWebServer (out of workspace) | Resolves Open Questions §8.1/§8.5. Prefer an explicit, documented URL/path form. **Open.** |
| F5 ✅ | Free-space query usable pre-sync | existing `filelist` `diskSize`/`diskUsed` (from `FileSystem::getFileSystemSize`) | No new endpoint needed; `filelist` already returns it cheaply. |
| F6 | *(Optional)* rename/move endpoint | `FileManager` + `FileSystem` | Enables atomic replace in `sync`. |
| F7 | *(Optional)* bulk delete endpoint | `FileManager` | Speeds delete-first phase on many-file web UIs. |

## 8. Open questions (to confirm against a real device / RaftWebServer)

Most of these are **resolved by the Phase 0 firmware work** (§7.1); they are listed so the
firmware changes can be specified precisely.

1. **`fileupload` destination** (→ F4): how is the target filesystem + path + filename specified —
   via URL path (`/api/fileupload/<fs>/<name>`), multipart `Content-Disposition` filename, or
   both? (RaftWebServer is outside this workspace.)
2. **Default filesystem segment**: does the API accept an empty `<fs>` segment (`//`) to mean
   default, or must the client send `local`/`sd` explicitly?
3. **Direct file download** (→ F3): rather than relying on whatever static-file serving exists,
   Phase 0 adds an explicit binary-safe `filesection` endpoint. Confirm whether any direct
   static serving also exists as a fallback.
4. **Folder semantics** (→ F1): `filelist` to gain `isDir`/`type`; confirm whether any mkdir
   support exists or is needed.
5. **Sub-folder upload** (→ F4): can `fileupload` create/target sub-folders on LittleFS, and how
   are `/` separators encoded in the upload filename?
6. **SD hot-plug**: behaviour of fs commands when `sd` is configured but no card is present.
7. **Per-file hash/CRC for sync** (→ F2): expose a content CRC/hash so `raft fs sync` can detect
   same-size edits without downloading. The device already computes a CRC during upload
   (`FileStreamGetCRCFnType`).
8. **Rename endpoint** (→ F6): enables atomic write-temp-then-rename replacement during `sync`.
9. **Bulk delete / free-space pre-check** (→ F5/F7): query free space and/or delete multiple
   files efficiently, to make the delete-first phase of `sync` fast on devices with many
   web-UI assets.

## 9. Summary of code touch-points

**Firmware first (Phase 0):**
- RaftCore `FileSystem` ([`FileSystem.cpp`](../../RaftCore/components/core/FileSystem/FileSystem.cpp)):
  add `isDir`/`type` and content hash to the file listing (F1/F2); add a ranged binary read
  helper exposure (F3); confirm free-space query (F5).
- RaftSysMods `FileManager` ([`FileManager.cpp`](../../RaftSysMods/components/FileManager/FileManager.cpp)):
  add `apiFileSection` (F3), optional `apiFileHash` (F2), confirm/document `fileupload`
  addressing (F4), optional rename/bulk-delete endpoints (F6/F7).

**CLI (Phases 1–3):**
- New: `src/app_fs.rs` (command logic), `src/http_upload.rs` (shared uploader extracted from
  `app_ota.rs`), optional `src/http_client.rs` (tiny GET helper).
- Edit: `src/main.rs` (add `Action::Fs` + dispatch), `src/app_ota.rs` (delegate to shared
  uploader), `src/raft_cli_utils.rs` (persist last device address; small size/format helpers).
