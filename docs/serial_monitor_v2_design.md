# Serial Monitor V2 — Design & Implementation Plan

## 1. Motivation

The current `serial_monitor` module works but has several issues that limit its usability and maintainability:

| # | Problem | Root Cause |
|---|---------|-----------|
| 1 | **Performance lag** — USB serial data from ESP32 falls behind | 100-byte read buffer, 1 ms sleep per loop iteration, Mutex contention on serial port shared between reader and writer threads, and channel overhead |
| 2 | **Gaps / dropped characters** when window is backgrounded | `crossterm::event::poll` blocks the main loop; if the OS deprioritises the process the `mpsc` channel fills or the serial port times out and data is lost before it can be drained |
| 3 | **No terminal resize handling** | `TerminalIO` captures `cols`/`rows` once at init and never updates them |
| 4 | **No colour-coded output** | `display_received_data` writes raw bytes; no log-level parsing |
| 5 | **Excessive complexity** | Three threads (serial-read, serial-write, main), an `Arc<Mutex<Box<dyn SerialPort>>>` shared between them, and lock contention at every read/write |
| 6 | **Fork dependency** | Uses `serialport-fix-stop-bits` crate — a modified fork of `serialport-rs` |
| 7 | **Missed ESC keystrokes** | `event::poll` with 10 ms timeout can miss short key-presses, especially when lock contention delays the poll |

Additionally, the current architecture makes it difficult to add higher-level features such as:

- Duplicate log-line summarisation (Chrome-console style)
- ESP32 crash-dump / backtrace decoding
- Filtering / searching through output
- Timestamps on lines

## 2. Design Goals

1. **Minimal external crates.** Use only the Rust standard library plus the absolute minimum:
   - A serial port crate (ideally the upstream `serialport` — evaluate whether the fork is still needed)
   - `crossterm` for terminal I/O in raw mode (already a dependency; lightweight and well-maintained)
   - `regex` for log-line parsing (already a dependency)
   - Nothing else new. All display, buffering, colour, and line-tracking logic will be hand-written.

2. **Predictable, low-latency data path.** The serial port reader must never compete with other threads for access to the port handle.

3. **Correct terminal resize handling.**

4. **Optional colour coding of ESP32 log output.**

5. **Simple, linear architecture** that is easy to reason about and extend.

6. **Feature parity** with the current serial monitor (command input, command history, serial reconnection, logging to file, WSL delegation).

7. **Extensibility** for future features (duplicate-line summarisation, crash-dump decoding, filtering, timestamps).

## 3. Architecture Overview

```
┌──────────────────────────────────────────────────────────┐
│                     Main Thread                          │
│                                                          │
│  ┌──────────┐    ┌──────────────┐    ┌───────────────┐   │
│  │ Terminal  │───>│  Dispatcher  │───>│  Serial Port  │   │
│  │  Input    │    │  (poll loop) │    │  (write)      │   │
│  └──────────┘    │              │    └───────────────┘   │
│                  │              │                        │
│  ┌──────────┐    │              │    ┌───────────────┐   │
│  │ Terminal  │<───│              │<───│  Serial Port  │   │
│  │  Output   │    │              │    │  (reader thd) │   │
│  └──────────┘    └──────────────┘    └───────────────┘   │
│                                                          │
│  ┌──────────────────────────────────────────────────┐    │
│  │              Line Processor                       │    │
│  │  (colour, dedup, crash decode, timestamps)        │    │
│  └──────────────────────────────────────────────────┘    │
│                                                          │
│  ┌──────────────┐  ┌──────────────┐                     │
│  │ Command      │  │  Log File    │                     │
│  │ History      │  │  Writer      │                     │
│  └──────────────┘  └──────────────┘                     │
└──────────────────────────────────────────────────────────┘
```

### 3.1 Thread Model — Exactly Two Threads

| Thread | Responsibility | Blocking? |
|--------|---------------|-----------|
| **Serial Reader** | Owns the read-half of the serial port. Reads into a large ring buffer. Sends chunks to the main thread over an `mpsc` channel. | Blocks on serial read with a short timeout (50–100 ms). |
| **Main** | Owns the terminal (raw mode). Polls `crossterm` events and the `mpsc` channel in a single select-style loop. Writes commands directly to the serial port (via a separate, non-shared write handle). Renders output. | Non-blocking poll loop with `crossterm::event::poll`. |

**Key simplification:** The serial port `write` handle is owned exclusively by the main thread — no Mutex, no write thread. Serial commands are short strings; writing them inline is negligible latency. The serial port `read` is done exclusively in the reader thread — again no Mutex.

### 3.2 Crate Usage

| Crate | Purpose | Status |
|-------|---------|--------|
| `serialport` (upstream) | Serial port open / read / write | **Evaluate** whether upstream now handles the stop-bits issue that prompted the fork. If yes, switch. If no, keep fork but document the specific issue. |
| `crossterm` | Raw terminal mode, key events, cursor control, colours, resize events | Already used |
| `regex` | Log-level parsing for colour coding | Already used |
| `chrono` | Timestamp formatting for log files | Already used |
| std `mpsc` | Channel from reader thread to main thread | Standard library |
| std `thread` | Reader thread | Standard library |
| std `fs` / `io` | File logging, command history | Standard library |

**No new crates are introduced.** The total external crate footprint for the serial monitor module is: `serialport` (or fork), `crossterm`, `regex`, `chrono`.

## 4. Detailed Module Design

The new implementation will live in a new source file `src/serial_monitor_v2.rs` (and optionally a sub-module directory `src/serial_monitor_v2/` if it grows). This allows the old and new implementations to coexist during development and testing.

### 4.1 `SerialReader` — Reader Thread Logic

```rust
// Pseudo-structure
struct SerialReader {
    port_name: String,
    baud_rate: u32,
    tx: mpsc::Sender<ReaderEvent>,
    running: Arc<AtomicBool>,
    no_reconnect: bool,
}

enum ReaderEvent {
    Data(Vec<u8>),       // Raw bytes from serial port
    Error(String),       // Error message (port disconnected, etc.)
    Reconnected,         // Successfully reconnected
}
```

**Behaviour:**

- Opens the serial port with a read timeout of 50 ms.
- Reads into a **4 KB stack buffer** (vs the current 100 bytes). Larger buffer = fewer syscalls = better throughput at high baud rates.
- On successful read, sends a `ReaderEvent::Data(bytes)` immediately via the channel. The `Vec<u8>` is allocated only for the actual bytes read (no wasted allocation).
- On timeout: loops immediately (no sleep).
- On error: sends `ReaderEvent::Error`, then attempts reconnection if `no_reconnect` is false. Backs off with short sleeps between reconnection attempts (100 ms, 200 ms, 500 ms, 1 s, capped at 2 s).
- **No Mutex.** The reader thread exclusively owns the read-half. On platforms where `serialport` doesn't support `try_clone()`, the reader thread will own the entire port and the main thread will send write-commands via a second `mpsc` channel to the reader, which will write inline between reads. (See §4.6 for the fallback path.)

### 4.2 `Display` — Terminal Output Engine

Replaces the current `TerminalIO`. Goals: handle resize, support colour, maintain a command-input line at the bottom.

```rust
struct Display {
    cols: u16,
    rows: u16,
    cursor_col: u16,         // Output cursor column (in the scrollable output area)
    cursor_row: u16,         // Output cursor row
    command_buffer: String,
    command_cursor_pos: usize,  // Position within command_buffer (for future left/right arrow support)
    is_error: bool,
    colour_enabled: bool,
    command_history: CommandHistory,
    line_processor: LineProcessor,
}
```

**Terminal layout:**

```
┌─────────────────────────────────────────┐
│  Scrollable output area                 │  rows 0 .. (rows-2)
│  (serial data, colour-coded)            │
│                                         │
├─────────────────────────────────────────┤
│ > command input                         │  row (rows-1)
└─────────────────────────────────────────┘
```

**Resize handling:**

- The main loop listens for `crossterm::event::Event::Resize(cols, rows)`.
- On resize, `Display` updates its `cols` and `rows`, clears the screen, and redraws the command prompt on the new last row.
- The output cursor position is clamped to the new dimensions.

**Colour coding:**

ESP-IDF log output follows the pattern:

```
I (12345) TAG: message     ← Info
W (12345) TAG: message     ← Warning
E (12345) TAG: message     ← Error
D (12345) TAG: message     ← Debug
V (12345) TAG: message     ← Verbose
```

The `LineProcessor` (§4.3) will parse complete lines and annotate them with a log level. The `Display` will apply colours:

| Level | Colour |
|-------|--------|
| Error (E) | Red |
| Warning (W) | Yellow |
| Info (I) | Green |
| Debug (D) | Cyan |
| Verbose (V) | White (default) |
| Unknown | White (default) |

Colour output uses `crossterm::style::SetForegroundColor` / `ResetColor`. This is optional and controlled by a flag (`--colour` / `--no-colour`).

**Output method:**

Instead of calling `cursor::position()` (which is a blocking ioctl / escape-sequence round-trip that adds latency), the `Display` will **track its own cursor position** arithmetically as it writes characters. This eliminates a significant source of latency and unpredictability in the current implementation.

### 4.3 `LineProcessor` — Line Buffering and Transformation

Sits between raw serial bytes and the `Display`. Operates on the main thread (no extra thread needed).

```rust
struct LineProcessor {
    partial_line: String,      // Accumulates bytes until a newline
    colour_enabled: bool,
}

struct ProcessedLine {
    text: String,
    level: LogLevel,
}

enum LogLevel {
    Error,
    Warning,
    Info,
    Debug,
    Verbose,
    Unknown,
}
```

**Behaviour:**

- Receives raw bytes from the reader channel.
- Splits on `\n` (handles `\r\n` and bare `\n`).
- For each complete line, determines `LogLevel` using a simple prefix check (a single-pass scan — no regex needed for the basic case):
  - Checks if the line starts with `E `, `W `, `I `, `D `, `V ` (with the standard ESP-IDF log format).
  - Falls back to `Unknown`.
- Passes `ProcessedLine` to `Display` for rendering.
- Incomplete lines are held in `partial_line` until the next batch of bytes arrives. If `colour_enabled` is false, partial lines can be flushed immediately to the terminal for lowest latency (matching current behaviour of writing raw bytes as they arrive).

**Future extension points** (not implemented in V2 initial release, but the architecture supports them):

- **Duplicate line summarisation:** Track the last N lines; if a line repeats, increment a counter and update the display in-place (e.g., `"I (xxx) TAG: message  [×42]"`).
- **Crash dump decoding:** Detect the `Backtrace:` or `Guru Meditation Error` patterns, capture the following addresses, and shell out to `addr2line` or `xtensa-esp32-elf-addr2line` to decode them.
- **Timestamps:** Prepend a local timestamp to each line.
- **Filtering:** Allow the user to type a filter command (e.g., `/TAG:WiFi`) that hides non-matching lines.

### 4.4 `CommandInput` — Key Handling

Extracted from the current `TerminalIO.handle_key_event` into the `Display` struct with improvements:

| Key | Action |
|-----|--------|
| Printable char | Append to command buffer |
| Backspace | Delete char before cursor |
| Delete | Delete char at cursor (new) |
| Left / Right arrow | Move cursor within command buffer (new) |
| Up / Down arrow | Command history navigation |
| Enter | Send command to serial port, add to history |
| Esc | Exit monitor |
| Ctrl+C / Ctrl+X | Exit monitor |
| Home | Move cursor to start of command (new) |
| End | Move cursor to end of command (new) |

**ESC key reliability fix:**

The current implementation misses ESC because `crossterm::event::poll` with a short timeout can be in the middle of processing serial output when ESC arrives, and the main loop only polls keyboard events once per iteration.

Fix: The main loop will *always* drain all pending keyboard events in each iteration before processing serial data. `crossterm::event::poll(Duration::ZERO)` in a tight inner loop until no more events are pending ensures no keystrokes are lost. Then the loop polls with a longer timeout (10–20 ms) for the next wakeup.

```rust
// Pseudocode for the main loop's input handling
loop {
    // 1. Drain ALL pending keyboard events (non-blocking)
    while crossterm::event::poll(Duration::ZERO)? {
        match event::read()? {
            Event::Key(ke) if ke.kind == KeyEventKind::Press => { /* handle */ }
            Event::Resize(c, r) => { display.handle_resize(c, r); }
            _ => {}
        }
    }

    // 2. Drain serial data from channel (non-blocking)
    while let Ok(event) = serial_rx.try_recv() {
        match event {
            ReaderEvent::Data(bytes) => { /* process and display */ }
            ReaderEvent::Error(msg) => { /* show error */ }
            ReaderEvent::Reconnected => { /* clear error */ }
        }
    }

    // 3. Block briefly waiting for next event (keyboard or channel)
    //    Use crossterm::event::poll with a short timeout
    let _ = crossterm::event::poll(Duration::from_millis(15));
}
```

### 4.5 `CommandHistory` — Reuse Existing Module

The existing `cmd_history.rs` module is well-written and has tests. It will be reused as-is. No changes needed.

### 4.6 Serial Port Write Strategy

**Primary path (when `try_clone()` is available):**

`serialport::SerialPort::try_clone()` returns a second handle to the same port. The main thread keeps the clone for writing; the reader thread keeps the original for reading. No Mutex needed.

```rust
let port = serialport::new(&port_name, baud_rate)
    .timeout(Duration::from_millis(50))
    .open()?;
let write_port = port.try_clone()?;
// port      → moved into reader thread
// write_port → stays on main thread
```

**Fallback path (if `try_clone()` is not supported on a platform or port type):**

Add a second `mpsc::Sender<Vec<u8>>` that the main thread uses to send write-commands to the reader thread. The reader thread writes inline between reads. This adds a tiny amount of latency to command sends but keeps the architecture Mutex-free.

### 4.7 Log File Writing

Reuse the existing `console_log.rs` module (`open_log_file`, `write_to_log`). The main thread writes to the log file after receiving data from the reader channel. This is simple and correct because writes are sequential on the main thread.

### 4.8 WSL Delegation

The existing `start_non_native()` function delegates to a Windows `raft.exe` process. This pattern will be preserved in V2 with the same logic.

## 5. Performance Analysis

### 5.1 Current Bottlenecks vs V2

| Bottleneck | Current | V2 |
|-----------|---------|-----|
| Read buffer size | 100 bytes | 4096 bytes |
| Sleep after read | 1 ms (`thread::sleep`) | None (blocks on serial timeout) |
| Serial port locking | `Arc<Mutex<Box<dyn SerialPort>>>` locked by both reader and writer | No Mutex — separate handles via `try_clone()` |
| Cursor position query | `cursor::position()` per output call (blocking escape sequence round-trip) | Self-tracked cursor position (zero I/O) |
| Terminal output per char batch | Clear line → move cursor → print → query position → move cursor → clear line → print prompt | Minimal: move cursor → print batch → move cursor → reprint prompt (only when prompt needs refresh) |
| Channel overhead | `mpsc` with String allocation per chunk | `mpsc` with `Vec<u8>` — same channel but fewer allocations (larger chunks) |
| Main loop cycle | ~11 ms minimum (10 ms poll + 1 ms serial sleep) | ~15 ms poll timeout, but serial data is processed immediately when available |

### 5.2 Expected Throughput

At 115200 baud, roughly 11,520 bytes/sec arrive. The current 100-byte buffer with 1 ms sleep handles this but with high overhead (115+ reads/sec, each triggering a Mutex lock + channel send + terminal redraw).

With a 4096-byte buffer, at 115200 baud we get ~3 reads/sec with full buffers, or more realistically short bursts read in 1–2 calls. At higher baud rates (921600 — common for ESP32 USB-Serial/JTAG), the current implementation almost certainly can't keep up; V2 should handle it comfortably.

## 6. Public API

The new module will expose the same two entry points as the current `serial_monitor`:

```rust
/// Start the serial monitor natively (direct serial port access).
pub fn start_native(
    app_folder: String,
    serial_port_name: Option<String>,
    baud_rate: u32,
    no_reconnect: bool,
    log: bool,
    log_folder: String,
    vid: Option<String>,
    history_file_name: String,
) -> Result<(), Box<dyn std::error::Error>>

/// Start the serial monitor via WSL delegation to raft.exe.
pub fn start_non_native(
    app_folder: String,
    port: Option<String>,
    baud: u32,
    no_reconnect: bool,
    log: bool,
    log_folder: String,
    vid: Option<String>,
) -> Result<(), Box<dyn std::error::Error>>
```

This ensures the switch from V1 to V2 requires only changing `mod serial_monitor` to `mod serial_monitor_v2` (and renaming later).

## 7. New Command-Line Flags

Added to `MonitorCmd` and `RunCmd`:

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--colour` / `--color` | bool | `true` | Enable colour-coded log output |
| `--no-colour` / `--no-color` | bool | — | Disable colour coding |
| `--timestamps` | bool | `false` | Prepend local timestamp to each line |

These are purely additive and do not break existing usage.

## 8. Implementation Plan

The implementation is divided into phases. Each phase produces a working (if incomplete) serial monitor that can be tested.

### Phase 1: Skeleton & Serial I/O (Core Loop)

**Files:** `src/serial_monitor_v2.rs`

1. Create `serial_monitor_v2.rs` with `start_native()` and `start_non_native()` stubs.
2. Implement `SerialReader`:
   - Open serial port.
   - `try_clone()` to get a write handle.
   - Spawn reader thread with 4 KB buffer, `mpsc::Sender<ReaderEvent>`.
   - Reconnection logic with backoff.
3. Implement the main loop skeleton:
   - Enable raw mode.
   - Poll `crossterm` events + drain `serial_rx` channel.
   - Write raw received bytes to stdout (no processing yet).
   - Handle ESC / Ctrl+C to exit.
   - Disable raw mode on exit.
4. Implement serial write: when Enter is pressed, write the command buffer to the write-port handle.
5. Wire up `start_non_native()` (copy from V1 — it's just a process spawn).

**Testable outcome:** Can open a serial port, display raw output, send commands, exit cleanly.

### Phase 2: Display & Command Input

**Files:** `src/serial_monitor_v2.rs`

1. Implement `Display` struct:
   - Track `cols`, `rows`, `cursor_col`, `cursor_row`.
   - Implement `print_output()` — writes serial data to the scrollable area, updates cursor tracking.
   - Implement `draw_prompt()` — draws the command buffer on the last row.
   - Implement `handle_resize()`.
2. Integrate `CommandHistory` (reuse `cmd_history.rs`).
3. Implement full key handling (printable chars, backspace, delete, arrows, up/down for history, home/end).
4. Handle `Event::Resize` in the main loop.

**Testable outcome:** Full interactive terminal with command history, proper resize handling.

### Phase 3: Line Processing & Colour

**Files:** `src/serial_monitor_v2.rs`

1. Implement `LineProcessor`:
   - Buffer partial lines.
   - Parse complete lines for log level.
2. Implement colour output in `Display`:
   - Map `LogLevel` to `crossterm` colours.
   - Apply colours per-line.
3. Add `--colour` / `--no-colour` flag handling.
4. When colour is disabled, flush partial lines immediately (zero-latency raw mode, matching V1 behaviour).

**Testable outcome:** Colour-coded ESP32 log output, with option to disable.

### Phase 4: Logging & Polish

**Files:** `src/serial_monitor_v2.rs`

1. Integrate `console_log.rs` for file logging.
2. Add `--timestamps` support (prepend timestamp in `LineProcessor`).
3. Add error/info display on the status line.
4. Comprehensive testing: reconnection, high-speed data, resize, WSL delegation.
5. Update `main.rs` to use `serial_monitor_v2` (behind a feature flag or as a direct replacement).

**Testable outcome:** Feature-complete replacement for V1.

### Phase 5: Future Enhancements (Post-V2 Launch)

These are explicitly **out of scope** for the initial V2 implementation but the architecture is designed to support them:

1. **Duplicate line summarisation:** Track ring buffer of recent lines in `LineProcessor`. When a duplicate is detected, update the count in-place on the terminal.
2. **ESP32 crash dump decoding:** Pattern-match `Guru Meditation Error` / `Backtrace:` lines in `LineProcessor`. Collect addresses, shell out to `addr2line` with the project's ELF file (path derived from `app_folder` + build artifacts).
3. **Line filtering:** User command (e.g., `/filter TAG:WiFi`) stored in `Display`, applied in `LineProcessor` to hide non-matching lines.
4. **Scroll-back buffer:** Store the last N lines in memory; allow Shift+PageUp/PageDown to scroll back through history.
5. **Split pane:** Show raw output in the top pane and decoded/filtered output in the bottom pane.

## 9. Migration Strategy

1. **Coexistence:** V2 lives in `serial_monitor_v2.rs` alongside the existing `serial_monitor.rs`. Both are compiled.
2. **Testing:** During development, add a `--monitor-v2` flag to `MonitorCmd` and `RunCmd` that selects the new implementation.
3. **Switchover:** Once V2 is validated, make it the default. Keep the `--monitor-v1` flag for a release or two as a fallback.
4. **Cleanup:** Remove `serial_monitor.rs` and the V1/V2 flag. Rename `serial_monitor_v2.rs` to `serial_monitor.rs`.

## 10. Risk Assessment

| Risk | Likelihood | Mitigation |
|------|-----------|------------|
| `try_clone()` not supported on some port types (e.g., pseudo-terminals on Linux) | Low | Fallback path via write-command channel to reader thread (§4.6) |
| Upstream `serialport` crate still has the stop-bits bug | Medium | Test with ESP32 USB-Serial/JTAG and CP2102. If bug persists, keep fork temporarily and file/track upstream issue. |
| Colour parsing misidentifies non-ESP-IDF output | Low | Colour is opt-in. Parser only triggers on well-known prefix patterns. Unknown lines get default colour. |
| Terminal emulation edge cases (Unicode, wide chars) | Medium | V2 does not attempt full terminal emulation. It passes through raw bytes (minus colour annotation). Wide-char cursor tracking can be added incrementally. |

## 11. File Structure Summary

```
src/
    serial_monitor.rs       ← existing V1 (unchanged during development)
    serial_monitor_v2.rs    ← new V2 implementation
    terminal_io.rs          ← existing (used by V1 and debug_remote; NOT used by V2)
    cmd_history.rs          ← existing (reused by V2)
    console_log.rs          ← existing (reused by V2)
    app_ports.rs            ← existing (reused by V2 for port selection)
    main.rs                 ← updated to conditionally use V2
```

## 12. Summary of Improvements Over V1

| Aspect | V1 | V2 |
|--------|-----|-----|
| Threads | 3 (read, write, main) | 2 (read, main) |
| Serial port sharing | `Arc<Mutex<Box<dyn SerialPort>>>` | `try_clone()` — no Mutex |
| Read buffer | 100 bytes | 4096 bytes |
| Inter-read sleep | 1 ms forced sleep | None (timeout-based blocking) |
| Cursor tracking | `cursor::position()` blocking call | Arithmetic self-tracking |
| Terminal resize | Not handled | `Event::Resize` handler |
| Colour output | Not supported | Optional, log-level-aware |
| ESC key reliability | Can be missed during lock contention | All key events drained first, non-blocking |
| Line processing | None (raw bytes) | Structured: level parsing, extensible pipeline |
| Extensibility | Difficult (threading, locks) | Easy (single-threaded pipeline on main thread) |
| New crates added | — | None |
