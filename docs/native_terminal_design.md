# Native Terminal Implementation Plan

## Goal

Replace the `crossterm` crate with a native terminal module that uses ANSI/VT100 escape sequences directly and platform-specific OS calls for raw mode and terminal size. This eliminates the dependency on crossterm and fixes the scroll-back buffer and blank-line issues caused by crossterm's `ScrollUp` command.

## Current crossterm Usage

Three files use crossterm:

| File | Usage |
|------|-------|
| `serial_monitor.rs` | Full terminal control: raw mode, cursor, clear, scroll, color, keyboard input, resize |
| `terminal_io.rs` | Display only: cursor, clear, color. Key types for `handle_key_event` signature |
| `app_debug_remote.rs` | Input only: `event::poll`, `event::read`, key matching, `disable_raw_mode`. Delegates display to `TerminalIO` |

## Architecture

### New module: `native_terminal.rs`

A single file (~430 lines) providing a `NativeTerminal` struct that encapsulates all terminal operations. This replaces both crossterm and the existing `terminal_io.rs`.

### Public API

```rust
/// Key codes returned by the input parser
pub enum KeyCode {
    Char(char),
    Enter,
    Backspace,
    Escape,
    Up,
    Down,
    Left,
    Right,
}

/// Modifier flags
pub struct Modifiers {
    pub ctrl: bool,
    pub alt: bool,
}

/// A parsed key event
pub struct KeyEvent {
    pub code: KeyCode,
    pub modifiers: Modifiers,
}

/// Terminal events
pub enum TermEvent {
    Key(KeyEvent),
    Resize(u16, u16),
}

/// Main terminal handle
pub struct NativeTerminal { ... }

impl NativeTerminal {
    pub fn new() -> Result<Self, io::Error>;   // saves original mode, enables raw mode + VT processing
    pub fn cleanup(&mut self);                  // restores original terminal mode

    // --- Screen control ---
    pub fn size(&self) -> (u16, u16);           // (cols, rows)
    pub fn clear_screen(&mut self);
    pub fn clear_line(&mut self);
    pub fn move_to(&mut self, col: u16, row: u16);
    pub fn set_scroll_region(&mut self, top: u16, bottom: u16);  // DECSTBM
    pub fn reset_scroll_region(&mut self);

    // --- Text styling ---
    pub fn set_color_yellow(&mut self);
    pub fn set_color_red(&mut self);
    pub fn set_color_green(&mut self);
    pub fn reset_color(&mut self);

    // --- Output ---
    pub fn write_str(&mut self, s: &str);
    pub fn flush(&mut self);

    // --- Input ---
    pub fn poll_event(&mut self, timeout: Duration) -> bool;
    pub fn read_event(&mut self) -> Option<TermEvent>;
}
```

### Drop impl

`NativeTerminal` should implement `Drop` to call `cleanup()` automatically, ensuring the terminal is always restored even on panic.

## Implementation Details

### 1. ANSI Escape Sequences (~60 lines)

All output control uses standard VT100/ANSI sequences written directly to stdout:

| Operation | Sequence | Notes |
|-----------|----------|-------|
| Move cursor | `\x1b[{row};{col}H` | 1-indexed |
| Clear screen | `\x1b[2J` | |
| Clear line | `\x1b[2K` | |
| Set scroll region | `\x1b[{top};{bottom}r` | DECSTBM, 1-indexed |
| Reset scroll region | `\x1b[r` | Resets to full screen |
| Foreground yellow | `\x1b[33m` | |
| Foreground red | `\x1b[31m` | |
| Foreground green | `\x1b[32m` | |
| Reset attributes | `\x1b[0m` | |

Implemented as simple helper methods that `write!` to a buffered stdout handle.

### 2. Raw Mode (~80 lines)

#### Unix (`#[cfg(unix)]`)

Use the `nix` crate (already a dependency — add `"term"` feature):

```rust
use nix::sys::termios::{tcgetattr, tcsetattr, SetArg, Termios};

// Save original termios
let orig = tcgetattr(stdin_fd)?;

// Make raw
let mut raw = orig.clone();
nix::sys::termios::cfmakeraw(&mut raw);
tcsetattr(stdin_fd, SetArg::TCSAFLUSH, &raw)?;

// Restore on cleanup
tcsetattr(stdin_fd, SetArg::TCSAFLUSH, &orig)?;
```

#### Windows (`#[cfg(windows)]`)

Use `windows-sys` or the transitive `winapi` crate:

```rust
use windows_sys::Win32::System::Console::*;

// Get current mode
let mut orig_in_mode = 0u32;
GetConsoleMode(stdin_handle, &mut orig_in_mode);

// Enable raw mode + VT input
let raw_mode = ENABLE_VIRTUAL_TERMINAL_INPUT | ENABLE_WINDOW_INPUT;
SetConsoleMode(stdin_handle, raw_mode);

// Enable VT processing on stdout
let mut orig_out_mode = 0u32;
GetConsoleMode(stdout_handle, &mut orig_out_mode);
SetConsoleMode(stdout_handle, orig_out_mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING);

// Restore on cleanup
SetConsoleMode(stdin_handle, orig_in_mode);
SetConsoleMode(stdout_handle, orig_out_mode);
```

With `ENABLE_VIRTUAL_TERMINAL_INPUT` enabled on Windows, arrow keys and other special keys arrive as VT escape sequences — the same as Unix. This means the input parser is cross-platform.

### 3. Terminal Size (~40 lines)

#### Unix

```rust
use nix::libc::{ioctl, winsize, TIOCGWINSZ};

let mut ws: winsize = unsafe { std::mem::zeroed() };
unsafe { ioctl(stdout_fd, TIOCGWINSZ, &mut ws) };
(ws.ws_col, ws.ws_row)
```

#### Windows

```rust
use windows_sys::Win32::System::Console::*;

let mut info: CONSOLE_SCREEN_BUFFER_INFO = unsafe { std::mem::zeroed() };
GetConsoleScreenBufferInfo(stdout_handle, &mut info);
let cols = (info.srWindow.Right - info.srWindow.Left + 1) as u16;
let rows = (info.srWindow.Bottom - info.srWindow.Top + 1) as u16;
```

### 4. Resize Detection (~50 lines)

#### Unix

Register a `SIGWINCH` handler that sets an `AtomicBool` flag. The `poll_event` / `read_event` methods check this flag and, if set, query the new size and return `TermEvent::Resize`.

Use `nix::sys::signal` or the `signal-hook` crate (lightweight, no-std compatible).

Alternatively, just re-query terminal size on every poll cycle — this is simpler and adequate given the 15ms poll interval.

#### Windows

With `ENABLE_WINDOW_INPUT` set on the console input handle, `ReadConsoleInput` returns `WINDOW_BUFFER_SIZE_EVENT` records when the window is resized. Since we're reading from stdin in VT mode, we can alternatively just re-query `GetConsoleScreenBufferInfo` periodically (same approach as the Unix "just check each iteration" strategy).

**Recommended approach**: Re-query terminal size every poll cycle. It's a cheap syscall and avoids the complexity of signal handling. If the size changed since last check, emit a `Resize` event.

### 5. Keyboard Input Parser (~200 lines)

This is the most complex part. Raw stdin delivers bytes that must be decoded into key events.

#### Reading

- **Unix**: Set stdin to non-blocking (`fcntl` with `O_NONBLOCK`), use `nix::poll::poll()` with the desired timeout, then `read()`.
- **Windows**: With `ENABLE_VIRTUAL_TERMINAL_INPUT`, `ReadFile` / `read()` on stdin delivers VT sequences. Use `WaitForSingleObject` with timeout for polling.

#### Parsing VT Escape Sequences

With VT input mode enabled on both platforms, the byte patterns are identical:

| Key | Bytes |
|-----|-------|
| Regular char | UTF-8 encoded bytes |
| Enter | `\r` (0x0D) |
| Backspace | `\x7f` (Unix) or `\x08` (Windows) |
| Escape | `\x1b` (solo, after timeout) |
| Ctrl+C | `\x03` |
| Ctrl+X | `\x18` |
| Up arrow | `\x1b[A` |
| Down arrow | `\x1b[B` |
| Right arrow | `\x1b[C` |
| Left arrow | `\x1b[D` |

#### Escape sequence disambiguation

When `\x1b` is received, it could be:
- The start of a multi-byte sequence (arrow key, etc.)
- A standalone Escape keypress

Strategy: after receiving `\x1b`, attempt a non-blocking read. If more bytes follow immediately (`[` then `A`-`D` etc.), parse as a sequence. If no bytes follow within ~1ms, treat as standalone Escape. This matches the standard behavior of `readline`, `vim`, etc.

#### Parser state machine

```
Start:
  \x03 → Key(Char('c'), ctrl=true)
  \x18 → Key(Char('x'), ctrl=true)
  \x0D → Key(Enter)
  \x7F / \x08 → Key(Backspace)
  \x1b → goto EscapeState
  0x20..0x7E → Key(Char(c))
  0xC0..0xFF → begin UTF-8 multi-byte decode

EscapeState:
  '[' → goto CSIState
  (timeout / other) → Key(Escape)

CSIState:
  'A' → Key(Up)
  'B' → Key(Down)
  'C' → Key(Right)
  'D' → Key(Left)
  (other) → discard or ignore
```

The parser consumes bytes from a small internal buffer (ring buffer or `VecDeque<u8>`) that is filled by the platform-specific read call.

## Scroll Region Strategy (Key Fix)

The critical fix for the scrollback and blank-line issues:

```
On init:
  1. Clear screen
  2. Set scroll region to rows 1..N-1   →  \x1b[1;{rows-1}r
  3. Move cursor to (0, 0)
  4. Draw prompt on row N (outside scroll region)

When printing serial output:
  1. Move cursor to the tracked output position (within the scroll region)
  2. Print the data directly (including \r\n as-is)
  3. Let the terminal handle scrolling naturally — when the cursor hits
     the bottom of the scroll region and a \n is printed, the terminal
     scrolls the region up and correctly adds lines to the scrollback buffer
  4. Save the cursor position
  5. Redraw the prompt on row N

On resize:
  1. Query new size
  2. Reset scroll region to new rows 1..N-1  →  \x1b[1;{rows-1}r
  3. Redraw prompt on new row N
```

This approach:
- Uses the terminal's native scroll mechanism → scrollback buffer works correctly
- The prompt row is outside the scroll region → no blank line between output and prompt
- No explicit `ScrollUp` commands needed

## File Changes

### New files

| File | Description |
|------|-------------|
| `src/native_terminal.rs` | ~430 lines. Complete terminal abstraction: raw mode, ANSI output, input parsing, size/resize |

### Modified files

| File | Change |
|------|--------|
| `Cargo.toml` | Remove `crossterm = "0.29.0"`. Add `"term"` and `"poll"` features to `nix`. Add `windows-sys` (Windows-only target dependency) or use existing transitive `winapi`. |
| `src/main.rs` | Add `mod native_terminal;` |
| `src/serial_monitor.rs` | Replace crossterm imports with `native_terminal`. Replace `Display` struct internals to use `NativeTerminal`. Use scroll region for output area. Remove `execute!` calls. |
| `src/terminal_io.rs` | Replace crossterm imports with `native_terminal` types. Use `NativeTerminal` for cursor/color/clear operations. Remove `execute!` calls and `cursor::position()`. |
| `src/app_debug_remote.rs` | Replace crossterm `event::poll`/`event::read`/key types with `native_terminal` equivalents. Replace `terminal::disable_raw_mode()` with `NativeTerminal::cleanup()`. |

### Removed dependency

- `crossterm = "0.29.0"` from `Cargo.toml`

## Dependency Changes

```toml
# Remove:
crossterm = "0.29.0"

# Modify (add features):
nix = { version = "0.31", features = ["user", "term", "poll"] }

# Add (Windows only):
[target.'cfg(windows)'.dependencies]
windows-sys = { version = "0.59", features = ["Win32_System_Console", "Win32_Foundation"] }
```

Note: `windows-sys` may already be a transitive dependency through `serialport-fix-stop-bits`. Check with `cargo tree -i windows-sys` to see if it can be reused directly or if an explicit dependency is needed.

## Implementation Order

1. **Create `native_terminal.rs`** with the full public API and platform-specific implementations
2. **Update `terminal_io.rs`** to use `NativeTerminal` instead of crossterm — this keeps `app_debug_remote.rs` working during the transition
3. **Update `serial_monitor.rs`** to use `NativeTerminal` with scroll regions
4. **Update `app_debug_remote.rs`** to use `NativeTerminal` for input polling
5. **Remove `crossterm` from `Cargo.toml`** and verify clean build
6. **Test** on Windows natively and under WSL

## Testing Checklist

- [ ] Serial monitor starts, displays output, accepts commands
- [ ] Scrollback buffer works correctly (mouse wheel / scrollbar scrolls through history in order)
- [ ] No blank line between output and prompt
- [ ] Terminal resize correctly adjusts layout
- [ ] Ctrl+C / Ctrl+X / Esc exits cleanly and restores terminal
- [ ] Command history (Up/Down arrows) works
- [ ] Colored prompt and error messages display correctly
- [ ] Works in VS Code integrated terminal
- [ ] Works in Windows Terminal
- [ ] Works under WSL (non-native path)
- [ ] `app_debug_remote` still works with TerminalIO
- [ ] No busy-waiting (CPU usage stays low)

## Risks and Mitigations

| Risk | Mitigation |
|------|------------|
| Windows VT input mode not available on older Windows | Windows 10 1809+ supports it. This is the same minimum as crossterm itself requires. |
| Escape sequence parsing edge cases | Only need to handle ~6 sequences (arrows, backspace, escape). Not a general-purpose terminal emulator. |
| Some terminal emulators may handle DECSTBM scroll regions differently | DECSTBM is universally supported (it's from VT100, 1978). VS Code terminal, Windows Terminal, xterm, iTerm2 all support it. |
| Losing crossterm's future bug fixes | The surface area is small and stable (VT100 hasn't changed in decades). Low maintenance burden. |
