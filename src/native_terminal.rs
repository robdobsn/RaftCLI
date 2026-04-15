// RaftCLI: Native terminal abstraction
// Rob Dobson 2024-2026
//
// Replaces crossterm with direct ANSI escape sequences and platform-specific
// OS calls for raw mode and terminal size. This gives correct scrollback
// buffer behavior and eliminates the crossterm dependency.

use std::io::{self, Write};
use std::time::Duration;

// ── Public types ──────────────────────────────────────────────────────────

/// Key codes returned by the input parser
#[derive(Debug, Clone, PartialEq)]
pub enum KeyCode {
    Char(char),
    Enter,
    Backspace,
    Delete,
    Escape,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    Insert,
}

/// Modifier flags
#[derive(Debug, Clone, Default)]
pub struct Modifiers {
    pub ctrl: bool,
}

/// A parsed key event
#[derive(Debug, Clone)]
pub struct KeyEvent {
    pub code: KeyCode,
    pub modifiers: Modifiers,
}

/// Terminal events
pub enum TermEvent {
    Key(KeyEvent),
    Resize(u16, u16),
}

// ── ANSI helpers (private) ────────────────────────────────────────────────

fn ansi_move_to(out: &mut impl Write, col: u16, row: u16) {
    // CSI row;col H — 1-indexed
    write!(out, "\x1b[{};{}H", row + 1, col + 1).unwrap();
}

fn ansi_clear_screen(out: &mut impl Write) {
    write!(out, "\x1b[2J").unwrap();
}

fn ansi_clear_line(out: &mut impl Write) {
    write!(out, "\x1b[2K").unwrap();
}

fn ansi_set_scroll_region(out: &mut impl Write, top: u16, bottom: u16) {
    // DECSTBM — 1-indexed
    write!(out, "\x1b[{};{}r", top + 1, bottom + 1).unwrap();
}

fn ansi_reset_scroll_region(out: &mut impl Write) {
    write!(out, "\x1b[r").unwrap();
}

/// DECSCUSR — set cursor shape. 1=block blink, 2=block steady, 5=bar blink, 6=bar steady
fn ansi_set_cursor_shape(out: &mut impl Write, shape: u8) {
    write!(out, "\x1b[{} q", shape).unwrap();
}

/// Reset cursor shape to terminal default (DECSCUSR 0)
fn ansi_reset_cursor_shape(out: &mut impl Write) {
    write!(out, "\x1b[0 q").unwrap();
}

/// DECTCEM — hide cursor
fn ansi_hide_cursor(out: &mut impl Write) {
    write!(out, "\x1b[?25l").unwrap();
}

/// DECTCEM — show cursor
fn ansi_show_cursor(out: &mut impl Write) {
    write!(out, "\x1b[?25h").unwrap();
}

fn ansi_fg_yellow(out: &mut impl Write) {
    write!(out, "\x1b[33m").unwrap();
}

fn ansi_fg_red(out: &mut impl Write) {
    write!(out, "\x1b[31m").unwrap();
}

fn ansi_fg_green(out: &mut impl Write) {
    write!(out, "\x1b[32m").unwrap();
}

fn ansi_reset_color(out: &mut impl Write) {
    write!(out, "\x1b[0m").unwrap();
}

// ── Platform: raw mode & terminal size ────────────────────────────────────

#[cfg(unix)]
mod platform {
    use nix::libc;
    use nix::sys::termios::{self, SetArg, Termios};
    use std::io;
    use std::os::fd::BorrowedFd;
    use std::os::unix::io::AsRawFd;
    use std::time::Duration;

    pub struct RawModeState {
        original: Termios,
        fd: i32,
    }

    pub fn enable_raw_mode() -> Result<RawModeState, io::Error> {
        let fd = io::stdin().as_raw_fd();
        let borrowed = unsafe { BorrowedFd::borrow_raw(fd) };
        let original = termios::tcgetattr(&borrowed)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        let mut raw = original.clone();
        termios::cfmakeraw(&mut raw);
        termios::tcsetattr(&borrowed, SetArg::TCSAFLUSH, &raw)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        Ok(RawModeState { original, fd })
    }

    pub fn disable_raw_mode(state: &RawModeState) {
        let borrowed = unsafe { BorrowedFd::borrow_raw(state.fd) };
        let _ = termios::tcsetattr(&borrowed, SetArg::TCSAFLUSH, &state.original);
    }

    pub fn terminal_size() -> (u16, u16) {
        unsafe {
            let mut ws: libc::winsize = std::mem::zeroed();
            if libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut ws) == 0
                && ws.ws_col > 0
                && ws.ws_row > 0
            {
                (ws.ws_col, ws.ws_row)
            } else {
                (80, 24)
            }
        }
    }

    /// Poll stdin for readability with the given timeout.
    /// Returns true if data is available.
    pub fn poll_stdin(timeout: Duration) -> bool {
        use nix::poll::{poll, PollFd, PollFlags, PollTimeout};
        let fd = io::stdin().as_raw_fd();
        let borrowed = unsafe { BorrowedFd::borrow_raw(fd) };
        let mut fds = [PollFd::new(borrowed, PollFlags::POLLIN)];
        let timeout_ms = timeout.as_millis() as u16;
        let poll_timeout = PollTimeout::from(timeout_ms);
        match poll(&mut fds, poll_timeout) {
            Ok(n) => n > 0,
            Err(_) => false,
        }
    }

    /// Non-blocking read from stdin. Returns number of bytes read (0 if nothing available).
    pub fn read_stdin(buf: &mut [u8]) -> usize {
        use std::io::Read;
        // Set non-blocking for this read
        let fd = io::stdin().as_raw_fd();
        unsafe {
            let flags = libc::fcntl(fd, libc::F_GETFL);
            libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
            let n = io::stdin().lock().read(buf).unwrap_or(0);
            libc::fcntl(fd, libc::F_SETFL, flags);
            n
        }
    }
}

#[cfg(windows)]
mod platform {
    use std::io;
    use std::time::Duration;

    use windows_sys::Win32::System::Console::*;
    use windows_sys::Win32::Foundation::HANDLE;

    pub struct RawModeState {
        stdin_handle: HANDLE,
        stdout_handle: HANDLE,
        original_in_mode: u32,
        original_out_mode: u32,
    }

    // HANDLE values are process-global constants from GetStdHandle — safe to send across threads
    unsafe impl Send for RawModeState {}

    fn get_std_handle(which: u32) -> HANDLE {
        unsafe { GetStdHandle(which) }
    }

    pub fn enable_raw_mode() -> Result<RawModeState, io::Error> {
        let stdin_handle = get_std_handle(STD_INPUT_HANDLE);
        let stdout_handle = get_std_handle(STD_OUTPUT_HANDLE);

        let mut original_in_mode: u32 = 0;
        let mut original_out_mode: u32 = 0;
        unsafe {
            if GetConsoleMode(stdin_handle, &mut original_in_mode) == 0 {
                return Err(io::Error::last_os_error());
            }
            if GetConsoleMode(stdout_handle, &mut original_out_mode) == 0 {
                return Err(io::Error::last_os_error());
            }

            // Raw input: enable VT input, window input; disable line input, echo, processed input
            let raw_in = ENABLE_VIRTUAL_TERMINAL_INPUT | ENABLE_WINDOW_INPUT;
            if SetConsoleMode(stdin_handle, raw_in) == 0 {
                return Err(io::Error::last_os_error());
            }

            // Enable VT processing on stdout
            let out_mode = original_out_mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING | DISABLE_NEWLINE_AUTO_RETURN;
            if SetConsoleMode(stdout_handle, out_mode) == 0 {
                // Try without DISABLE_NEWLINE_AUTO_RETURN (not available on older builds)
                let out_mode = original_out_mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING;
                if SetConsoleMode(stdout_handle, out_mode) == 0 {
                    return Err(io::Error::last_os_error());
                }
            }
        }

        Ok(RawModeState {
            stdin_handle,
            stdout_handle,
            original_in_mode,
            original_out_mode,
        })
    }

    pub fn disable_raw_mode(state: &RawModeState) {
        unsafe {
            SetConsoleMode(state.stdin_handle, state.original_in_mode);
            SetConsoleMode(state.stdout_handle, state.original_out_mode);
        }
    }

    pub fn terminal_size() -> (u16, u16) {
        let handle = get_std_handle(STD_OUTPUT_HANDLE);
        unsafe {
            let mut info: CONSOLE_SCREEN_BUFFER_INFO = std::mem::zeroed();
            if GetConsoleScreenBufferInfo(handle, &mut info) != 0 {
                let cols = (info.srWindow.Right - info.srWindow.Left + 1) as u16;
                let rows = (info.srWindow.Bottom - info.srWindow.Top + 1) as u16;
                (cols, rows)
            } else {
                (80, 24)
            }
        }
    }

    /// Poll stdin for readability with the given timeout.
    pub fn poll_stdin(timeout: Duration) -> bool {
        use windows_sys::Win32::Foundation::WAIT_OBJECT_0;
        use windows_sys::Win32::System::Threading::WaitForSingleObject;
        let handle = get_std_handle(STD_INPUT_HANDLE);
        let ms = timeout.as_millis() as u32;
        let result = unsafe { WaitForSingleObject(handle, ms) };
        result == WAIT_OBJECT_0
    }

    /// Non-blocking read from stdin. Returns number of bytes read.
    /// Discards non-key console events (focus, mouse, buffer-size) that would
    /// otherwise cause a blocking read on stdin.
    pub fn read_stdin(buf: &mut [u8]) -> usize {
        use std::io::Read;
        let handle = get_std_handle(STD_INPUT_HANDLE);

        // Drain any non-key events that would cause stdin.read() to block
        loop {
            let mut record: INPUT_RECORD = unsafe { std::mem::zeroed() };
            let mut count: u32 = 0;
            let ok = unsafe { PeekConsoleInputW(handle, &mut record, 1, &mut count) };
            if ok == 0 || count == 0 {
                return 0; // Nothing in the buffer
            }
            if record.EventType as u32 == KEY_EVENT {
                // It's a key event — check if it's a key-down with a real character
                let key_event = unsafe { record.Event.KeyEvent };
                if key_event.bKeyDown != 0 {
                    break; // Real key-down, proceed to read
                }
                // Key-up event — consume and discard
                unsafe { ReadConsoleInputW(handle, &mut record, 1, &mut count) };
            } else {
                // Non-key event (focus, mouse, buffer-size) — consume and discard
                unsafe { ReadConsoleInputW(handle, &mut record, 1, &mut count) };
            }
        }

        io::stdin().lock().read(buf).unwrap_or(0)
    }
}

// ── Input parser ──────────────────────────────────────────────────────────

struct InputParser {
    buf: Vec<u8>,
}

impl InputParser {
    fn new() -> Self {
        Self { buf: Vec::with_capacity(64) }
    }

    /// Feed raw bytes from stdin into the parser buffer.
    fn feed(&mut self, data: &[u8]) {
        self.buf.extend_from_slice(data);
    }

    /// Try to parse the next key event from the buffer.
    /// Returns None if the buffer doesn't contain a complete sequence.
    fn next_event(&mut self) -> Option<KeyEvent> {
        if self.buf.is_empty() {
            return None;
        }

        let b = self.buf[0];
        match b {
            // Ctrl+C
            0x03 => {
                self.buf.remove(0);
                Some(KeyEvent {
                    code: KeyCode::Char('c'),
                    modifiers: Modifiers { ctrl: true },
                })
            }
            // Ctrl+X
            0x18 => {
                self.buf.remove(0);
                Some(KeyEvent {
                    code: KeyCode::Char('x'),
                    modifiers: Modifiers { ctrl: true },
                })
            }
            // Enter (CR)
            0x0D => {
                self.buf.remove(0);
                Some(KeyEvent {
                    code: KeyCode::Enter,
                    modifiers: Modifiers::default(),
                })
            }
            // Newline (LF) — also treat as Enter
            0x0A => {
                self.buf.remove(0);
                Some(KeyEvent {
                    code: KeyCode::Enter,
                    modifiers: Modifiers::default(),
                })
            }
            // Backspace (DEL on Unix, BS on some Windows)
            0x7F | 0x08 => {
                self.buf.remove(0);
                Some(KeyEvent {
                    code: KeyCode::Backspace,
                    modifiers: Modifiers::default(),
                })
            }
            // Escape or escape sequence
            0x1B => {
                if self.buf.len() >= 3 && self.buf[1] == b'[' {
                    // CSI sequence — find the terminating byte (alphabetic or ~)
                    let mut end = 2;
                    while end < self.buf.len()
                        && !self.buf[end].is_ascii_alphabetic()
                        && self.buf[end] != b'~'
                    {
                        end += 1;
                    }
                    if end >= self.buf.len() {
                        // Incomplete sequence — wait for more data
                        return None;
                    }

                    let terminator = self.buf[end];
                    // Collect the parameter bytes between '[' and the terminator
                    let params: Vec<u8> = self.buf[2..end].to_vec();
                    // Consume the entire sequence
                    self.buf.drain(..=end);

                    // Parse semicolon-separated numeric parameters (e.g. "1;5")
                    let parts: Vec<u16> = params
                        .split(|&b| b == b';')
                        .map(|p| {
                            p.iter().fold(0u16, |acc, &b| {
                                acc.wrapping_mul(10).wrapping_add((b.wrapping_sub(b'0')) as u16)
                            })
                        })
                        .collect();

                    // Detect modifier from second parameter (xterm: 2=Shift, 3=Alt, 5=Ctrl, etc.)
                    let modifiers = if parts.len() >= 2 {
                        Modifiers {
                            ctrl: parts[1] == 5 || parts[1] == 6, // 5=Ctrl, 6=Ctrl+Shift
                        }
                    } else {
                        Modifiers::default()
                    };

                    let code = match terminator {
                        // Arrow keys and Home/End letter-terminated
                        b'A' => Some(KeyCode::Up),
                        b'B' => Some(KeyCode::Down),
                        b'C' => Some(KeyCode::Right),
                        b'D' => Some(KeyCode::Left),
                        b'H' => Some(KeyCode::Home),
                        b'F' => Some(KeyCode::End),
                        // Tilde-terminated sequences
                        b'~' => match parts.first().copied().unwrap_or(0) {
                            1 | 7 => Some(KeyCode::Home),
                            2 => Some(KeyCode::Insert),
                            3 => Some(KeyCode::Delete),
                            4 | 8 => Some(KeyCode::End),
                            _ => None,
                        },
                        _ => None,
                    };

                    if let Some(kc) = code {
                        return Some(KeyEvent {
                            code: kc,
                            modifiers,
                        });
                    }
                    // Unknown CSI sequence — already consumed, try next
                    return self.next_event();
                } else if self.buf.len() >= 2 && self.buf[1] == b'[' {
                    // We have \x1b[ but nothing after — incomplete, wait
                    return None;
                } else if self.buf.len() == 1 {
                    // Lone ESC — could be standalone or start of sequence.
                    // We'll need more data to decide. The caller should
                    // provide a small delay and re-check.
                    return None;
                } else {
                    // \x1b followed by something that isn't '[' — standalone Escape
                    self.buf.remove(0);
                    Some(KeyEvent {
                        code: KeyCode::Escape,
                        modifiers: Modifiers::default(),
                    })
                }
            }
            // Regular printable ASCII
            0x20..=0x7E => {
                self.buf.remove(0);
                Some(KeyEvent {
                    code: KeyCode::Char(b as char),
                    modifiers: Modifiers::default(),
                })
            }
            // UTF-8 multi-byte sequences
            0xC0..=0xFF => {
                let expected_len = if b & 0xE0 == 0xC0 {
                    2
                } else if b & 0xF0 == 0xE0 {
                    3
                } else if b & 0xF8 == 0xF0 {
                    4
                } else {
                    // Invalid leading byte — skip
                    self.buf.remove(0);
                    return self.next_event();
                };
                if self.buf.len() < expected_len {
                    return None; // Incomplete UTF-8 — wait for more
                }
                let bytes: Vec<u8> = self.buf.drain(..expected_len).collect();
                if let Ok(s) = std::str::from_utf8(&bytes) {
                    if let Some(ch) = s.chars().next() {
                        return Some(KeyEvent {
                            code: KeyCode::Char(ch),
                            modifiers: Modifiers::default(),
                        });
                    }
                }
                // Invalid UTF-8, skip and try again
                self.next_event()
            }
            // Ctrl+A through Ctrl+Z (0x01-0x1A) other than the ones handled above
            0x01..=0x1A => {
                self.buf.remove(0);
                let ch = (b + b'a' - 1) as char;
                Some(KeyEvent {
                    code: KeyCode::Char(ch),
                    modifiers: Modifiers { ctrl: true },
                })
            }
            // Other control characters — skip
            _ => {
                self.buf.remove(0);
                self.next_event()
            }
        }
    }

    /// Returns true if the buffer has a lone ESC that may be a standalone
    /// Escape key, but we need to wait briefly to see if more bytes arrive.
    fn has_pending_escape(&self) -> bool {
        self.buf.len() == 1 && self.buf[0] == 0x1B
    }
}

// ── NativeTerminal ────────────────────────────────────────────────────────

pub struct NativeTerminal {
    raw_state: Option<platform::RawModeState>,
    parser: InputParser,
    last_cols: u16,
    last_rows: u16,
}

impl NativeTerminal {
    /// Create a new NativeTerminal, enable raw mode, and enable VT processing.
    pub fn new() -> Result<Self, io::Error> {
        let raw_state = platform::enable_raw_mode()?;
        let (cols, rows) = platform::terminal_size();
        // Set cursor to steady bar (beam) for a modern editing feel
        let mut out = io::stdout();
        ansi_set_cursor_shape(&mut out, 6);
        out.flush().unwrap();
        Ok(Self {
            raw_state: Some(raw_state),
            parser: InputParser::new(),
            last_cols: cols,
            last_rows: rows,
        })
    }

    /// Restore the terminal to its original state.
    pub fn cleanup(&mut self) {
        let mut out = io::stdout();
        // Ensure cursor is visible and shape is restored
        ansi_show_cursor(&mut out);
        ansi_reset_cursor_shape(&mut out);
        // Move cursor to the bottom of the screen before resetting,
        // so the shell prompt appears at the bottom after exit.
        let (_, rows) = platform::terminal_size();
        ansi_reset_scroll_region(&mut out);
        ansi_move_to(&mut out, 0, rows.saturating_sub(1));
        // Write a newline to ensure the cursor is on a fresh line
        write!(out, "\r\n").unwrap_or_default();
        out.flush().unwrap_or_default();

        if let Some(ref state) = self.raw_state {
            platform::disable_raw_mode(state);
        }
        self.raw_state = None;
    }

    // ── Size ──

    /// Query current terminal size.
    pub fn size(&self) -> (u16, u16) {
        platform::terminal_size()
    }

    // ── Screen control ──

    pub fn clear_screen(&mut self) {
        let mut out = io::stdout();
        ansi_clear_screen(&mut out);
        ansi_move_to(&mut out, 0, 0);
        out.flush().unwrap();
    }

    pub fn clear_line(&mut self) {
        let mut out = io::stdout();
        ansi_clear_line(&mut out);
        out.flush().unwrap();
    }

    pub fn move_to(&mut self, col: u16, row: u16) {
        let mut out = io::stdout();
        ansi_move_to(&mut out, col, row);
        out.flush().unwrap();
    }

    /// Set the scrolling region (DECSTBM). Rows outside this region are fixed.
    pub fn set_scroll_region(&mut self, top: u16, bottom: u16) {
        let mut out = io::stdout();
        ansi_set_scroll_region(&mut out, top, bottom);
        out.flush().unwrap();
    }

    pub fn reset_scroll_region(&mut self) {
        let mut out = io::stdout();
        ansi_reset_scroll_region(&mut out);
        out.flush().unwrap();
    }

    // ── Text styling ──

    pub fn set_color_yellow(&mut self) {
        let mut out = io::stdout();
        ansi_fg_yellow(&mut out);
        out.flush().unwrap();
    }

    pub fn set_color_red(&mut self) {
        let mut out = io::stdout();
        ansi_fg_red(&mut out);
        out.flush().unwrap();
    }

    pub fn set_color_green(&mut self) {
        let mut out = io::stdout();
        ansi_fg_green(&mut out);
        out.flush().unwrap();
    }

    pub fn reset_color(&mut self) {
        let mut out = io::stdout();
        ansi_reset_color(&mut out);
        out.flush().unwrap();
    }

    // ── Output ──

    pub fn write_str(&mut self, s: &str) {
        let mut out = io::stdout();
        write!(out, "{}", s).unwrap();
        out.flush().unwrap();
    }

    #[allow(dead_code)]
    pub fn write_bytes(&mut self, b: &[u8]) {
        let mut out = io::stdout();
        out.write_all(b).unwrap();
        out.flush().unwrap();
    }

    pub fn flush(&mut self) {
        io::stdout().flush().unwrap();
    }

    /// Hide the terminal cursor (DECTCEM).
    pub fn hide_cursor(&mut self) {
        let mut out = io::stdout();
        ansi_hide_cursor(&mut out);
        out.flush().unwrap();
    }

    /// Show the terminal cursor (DECTCEM).
    pub fn show_cursor(&mut self) {
        let mut out = io::stdout();
        ansi_show_cursor(&mut out);
        out.flush().unwrap();
    }

    // ── Input ──

    /// Poll for terminal events with the given timeout.
    /// Returns true if at least one event is available via `read_event()`.
    pub fn poll_event(&mut self, timeout: Duration) -> bool {
        // First check if parser already has buffered data
        if !self.parser.buf.is_empty() {
            return true;
        }

        // Check for resize (cheap syscall)
        let (cols, rows) = platform::terminal_size();
        if cols != self.last_cols || rows != self.last_rows {
            return true; // Resize detected
        }

        // Poll stdin
        platform::poll_stdin(timeout)
    }

    /// Read the next terminal event. Call `poll_event` first.
    /// Returns None if no event is available.
    pub fn read_event(&mut self) -> Option<TermEvent> {
        // Check for resize
        let (cols, rows) = platform::terminal_size();
        if cols != self.last_cols || rows != self.last_rows {
            self.last_cols = cols;
            self.last_rows = rows;
            return Some(TermEvent::Resize(cols, rows));
        }

        // Read raw bytes from stdin
        let mut raw = [0u8; 64];
        let n = platform::read_stdin(&mut raw);
        if n > 0 {
            self.parser.feed(&raw[..n]);
        }

        // Handle lone ESC disambiguation: wait briefly for more bytes
        if self.parser.has_pending_escape() {
            if platform::poll_stdin(Duration::from_millis(2)) {
                let n = platform::read_stdin(&mut raw);
                if n > 0 {
                    self.parser.feed(&raw[..n]);
                }
            }
            // If still a lone ESC after the wait, force it as standalone
            if self.parser.has_pending_escape() {
                self.parser.buf.clear();
                return Some(TermEvent::Key(KeyEvent {
                    code: KeyCode::Escape,
                    modifiers: Modifiers::default(),
                }));
            }
        }

        // Parse the next key event
        self.parser.next_event().map(TermEvent::Key)
    }
}

impl Drop for NativeTerminal {
    fn drop(&mut self) {
        self.cleanup();
    }
}
