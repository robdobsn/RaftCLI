// RaftCLI: Serial Monitor
// Rob Dobson 2024-2026
//
// Two-thread architecture:
//   - Reader thread: owns the read-half of the serial port, sends data via mpsc channel
//   - Main thread: owns the terminal + write-half of the serial port, no Mutex needed
//
// Uses DECSTBM scroll regions for correct scrollback buffer behavior.

use serialport::{new as serial_new, SerialPort};
use std::collections::VecDeque;
use std::io::{self, Write};
use std::process::{Command, Stdio};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};
use std::thread;
use std::time::Duration;

use chrono::Local;

use crate::app_ports::{select_most_likely_port, PortsCmd};
use crate::console_log::{open_log_file, write_to_log};
use crate::line_editor::{LineEditAction, LineEditor};
use crate::native_terminal::{NativeTerminal, TermEvent};

// Size of the serial read buffer
const SERIAL_READ_BUF_SIZE: usize = 4096;

// Maximum number of output lines to retain for redrawing after resize
const MAX_OUTPUT_LINES: usize = 2000;

// Mode for injecting wall-clock receive timestamps into the output stream
#[derive(Clone, Debug)]
pub enum RxTimestampMode {
    /// Inject timestamp before the first byte of each new line
    First,
    /// Inject timestamp at the end of each line (before the newline)
    Eol,
}

// Events sent from the reader thread to the main thread
enum ReaderEvent {
    Data(Vec<u8>),
    Error(String),
    Reconnected,
}

// Display state — uses NativeTerminal with DECSTBM scroll region
struct Display {
    term: NativeTerminal,
    cols: u16,
    rows: u16,
    output_col: u16,
    output_row: u16,
    is_error: bool,
    editor: LineEditor,
    /// True when the last character written filled the final column, leaving
    /// the terminal in deferred-wrap state. Resolved on the next character.
    pending_wrap: bool,
    /// Ring buffer of completed output lines for redrawing after resize.
    /// The last entry may be a partial (unterminated) line.
    line_buffer: VecDeque<String>,
    /// Accumulates the current (possibly incomplete) line.
    current_line: String,
    /// Wall-clock timestamp injection mode.
    rx_timestamps: Option<RxTimestampMode>,
    /// True when the next visible character begins a new line (used by First mode).
    at_line_start: bool,
}

impl Display {
    fn new(history_file_path: &str, rx_timestamps: Option<RxTimestampMode>) -> Display {
        let term = NativeTerminal::new().expect("Failed to initialize terminal");
        let (cols, rows) = term.size();
        Display {
            term,
            cols,
            rows,
            output_col: 0,
            output_row: 0,
            is_error: false,
            editor: LineEditor::new(history_file_path),
            pending_wrap: false,
            line_buffer: VecDeque::new(),
            current_line: String::new(),
            rx_timestamps,
            at_line_start: true,
        }
    }

    /// Pre-process `data` by injecting wall-clock timestamps according to `rx_timestamps`.
    /// Mutates `at_line_start` to track position across successive calls.
    fn inject_timestamps(&mut self, data: &str) -> String {
        let mut out = String::with_capacity(data.len() + 20);
        match &self.rx_timestamps {
            Some(RxTimestampMode::First) => {
                for ch in data.chars() {
                    if self.at_line_start && ch != '\r' && ch != '\n' {
                        out.push_str(&Local::now().format("[%H:%M:%S%.3f] ").to_string());
                        self.at_line_start = false;
                    }
                    if ch == '\n' {
                        self.at_line_start = true;
                    }
                    out.push(ch);
                }
            }
            Some(RxTimestampMode::Eol) => {
                for ch in data.chars() {
                    if ch == '\n' {
                        out.push_str(&Local::now().format(" [%H:%M:%S%.3f]").to_string());
                    }
                    out.push(ch);
                }
            }
            None => {
                return data.to_string();
            }
        }
        out
    }

    fn init(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let (cols, rows) = self.term.size();
        self.cols = cols;
        self.rows = rows;

        self.term.clear_screen();

        // Set scroll region to all rows except the last (prompt row)
        if self.rows > 1 {
            self.term.set_scroll_region(0, self.rows - 2);
        }
        self.term.move_to(0, 0);
        self.pending_wrap = false;
        Ok(())
    }

    fn handle_resize(&mut self, cols: u16, rows: u16) {
        self.cols = cols;
        self.rows = rows;
        // Brief delay to let the terminal emulator finish re-layout before
        // clearing — otherwise the clear can fire before the new size is
        // visually applied, leaving stale content in the expanded area.
        std::thread::sleep(std::time::Duration::from_millis(50));
        // Re-read size in case it changed during the delay
        let (cols, rows) = self.term.size();
        self.cols = cols;
        self.rows = rows;
        // Clear and redraw from the line buffer
        self.term.reset_scroll_region();
        self.term.clear_screen();
        self.output_row = 0;
        self.output_col = 0;
        self.pending_wrap = false;
        if self.rows > 1 {
            self.term.set_scroll_region(0, self.rows - 2);
        }
        self.term.move_to(0, 0);
        self.redraw_from_buffer();
        self.draw_prompt();
    }

    /// Write serial output data using the scroll region.
    /// The terminal handles scrolling naturally, which correctly fills the
    /// scrollback buffer.
    /// Does NOT redraw the prompt — call `draw_prompt` once after draining
    /// a batch of serial events to avoid redundant escape sequences.
    fn print_output(&mut self, data: &str) {
        self.is_error = false;

        // Inject wall-clock receive timestamps if enabled.
        // We bind to a local to extend lifetime before borrowing as &str.
        let owned;
        let data: &str = if self.rx_timestamps.is_some() {
            owned = self.inject_timestamps(data);
            &owned
        } else {
            data
        };

        // Hide cursor while updating the scroll region so it doesn't
        // flicker in the output area
        self.term.hide_cursor();

        // Buffer the data for redraw-on-resize.
        // Ignore \r for buffering — serial devices typically send \r\n and
        // the \r would clear the line content before \n commits it.
        for ch in data.chars() {
            if ch == '\n' {
                self.line_buffer.push_back(std::mem::take(&mut self.current_line));
                if self.line_buffer.len() > MAX_OUTPUT_LINES {
                    self.line_buffer.pop_front();
                }
            } else if ch != '\r' && !ch.is_control() {
                self.current_line.push(ch);
            }
        }

        // Clear the prompt line
        self.term.reset_scroll_region();
        self.term.move_to(0, self.rows - 1);
        self.term.clear_line();

        // Restore scroll region and move to output position
        if self.rows > 1 {
            self.term.set_scroll_region(0, self.rows - 2);
        }
        self.term.move_to(self.output_col, self.output_row);

        let max_output_row = self.rows.saturating_sub(2);

        // Build a single output buffer to minimise syscalls
        let mut out_buf = String::with_capacity(data.len() + 32);
        for ch in data.chars() {
            match ch {
                '\n' => {
                    self.pending_wrap = false;
                    self.output_col = 0;
                    if self.output_row >= max_output_row {
                        // At bottom of scroll region — write \n which makes terminal scroll
                        out_buf.push('\n');
                        // Row stays at max (terminal scrolled the region up)
                    } else {
                        self.output_row += 1;
                        out_buf.push('\n');
                    }
                }
                '\r' => {
                    self.pending_wrap = false;
                    self.output_col = 0;
                    out_buf.push('\r');
                }
                c if !c.is_control() => {
                    if self.pending_wrap {
                        // Resolve deferred line wrap: advance to the next row.
                        // \r cancels the terminal's deferred-wrap state (moving
                        // cursor to column 0), then \n advances one row
                        // (scrolling if at the bottom of the scroll region).
                        out_buf.push_str("\r\n");
                        self.output_col = 0;
                        if self.output_row < max_output_row {
                            self.output_row += 1;
                        }
                        self.pending_wrap = false;
                    }
                    out_buf.push(c);
                    self.output_col += 1;
                    if self.output_col >= self.cols {
                        // Cursor is at the last column in deferred-wrap state.
                        // Don't advance yet — the next character will resolve it.
                        self.output_col = self.cols - 1;
                        self.pending_wrap = true;
                    }
                }
                _ => {
                    // Skip other control characters
                }
            }
        }
        let mut out = io::stdout();
        out.write_all(out_buf.as_bytes()).unwrap();
        out.flush().unwrap();
        // Prompt is redrawn by the caller after the full serial drain batch.
    }

    /// Redraw the output area from the line buffer after a resize.
    fn redraw_from_buffer(&mut self) {
        let output_rows = self.rows.saturating_sub(1) as usize; // rows available for output
        if output_rows == 0 {
            return;
        }
        let cols = self.cols as usize;
        if cols == 0 {
            return;
        }

        // Collect lines to display: each stored line may wrap across multiple
        // terminal rows at the new width. Work backwards from the newest lines.
        let mut screen_lines: Vec<&str> = Vec::new();
        let mut total_rows_used: usize = 0;

        // Include the current partial line if non-empty
        let partial = if !self.current_line.is_empty() {
            Some(self.current_line.as_str())
        } else {
            None
        };

        let iter_partial = partial.into_iter();
        let iter_completed = self.line_buffer.iter().rev().map(|s| s.as_str());

        // Walk backwards: partial line first (it's newest), then completed lines newest-to-oldest
        for line in iter_partial.chain(iter_completed) {
            let line_rows = if line.is_empty() {
                1
            } else {
                (line.chars().count() + cols - 1) / cols
            };
            if total_rows_used + line_rows > output_rows {
                break;
            }
            total_rows_used += line_rows;
            screen_lines.push(line);
        }

        // Reverse so oldest is first (top of screen)
        screen_lines.reverse();

        // Write them out within the scroll region
        self.term.move_to(0, 0);
        let mut out = io::stdout();
        let max_output_row = self.rows.saturating_sub(2);
        self.output_row = 0;
        self.output_col = 0;

        for (i, line) in screen_lines.iter().enumerate() {
            write!(out, "{}", line).unwrap();
            // Track output position
            let char_count = line.chars().count() as u16;
            self.output_col = char_count % self.cols;
            let rows_used = if char_count == 0 { 0 } else { char_count / self.cols };
            self.output_row = self.output_row.saturating_add(rows_used);
            if self.output_row > max_output_row {
                self.output_row = max_output_row;
            }

            // Add newline between completed lines (not after the last if it's the partial line)
            let is_last = i == screen_lines.len() - 1;
            let is_partial = is_last && !self.current_line.is_empty();
            if !is_last || !is_partial {
                write!(out, "\r\n").unwrap();
                self.output_col = 0;
                if self.output_row < max_output_row {
                    self.output_row += 1;
                }
            }
        }
        out.flush().unwrap();
    }

    fn draw_prompt(&mut self) {
        // Temporarily reset scroll region to write on the fixed bottom row
        self.term.reset_scroll_region();
        self.term.move_to(0, self.rows - 1);
        self.term.clear_line();
        self.term.set_color_yellow();
        let buf = self.editor.buffer_str();
        self.term.write_str(&format!("> {}", buf));
        self.term.reset_color();
        // Restore scroll region first (DECSTBM can reset cursor position)
        if self.rows > 1 {
            self.term.set_scroll_region(0, self.rows - 2);
        }
        // Position cursor on the prompt line *after* restoring scroll region
        let cursor_col = 2 + self.editor.cursor_pos() as u16;
        self.term.move_to(cursor_col, self.rows - 1);
        self.term.show_cursor();
        self.term.flush();
    }

    fn show_error(&mut self, msg: &str) {
        self.term.reset_scroll_region();
        self.term.move_to(0, self.rows - 1);
        self.term.clear_line();
        self.term.set_color_red();
        self.term.write_str(&format!("! {}", msg));
        self.term.reset_color();
        self.term.flush();
        if self.rows > 1 {
            self.term.set_scroll_region(0, self.rows - 2);
        }
        self.is_error = true;
    }

    fn show_info(&mut self, msg: &str) {
        self.term.reset_scroll_region();
        self.term.move_to(0, self.rows - 1);
        self.term.clear_line();
        self.term.set_color_green();
        self.term.write_str(&format!("> {}", msg));
        self.term.reset_color();
        self.term.flush();
        if self.rows > 1 {
            self.term.set_scroll_region(0, self.rows - 2);
        }
    }

    /// Handle a key event. Returns false if the monitor should exit.
    fn handle_key_event(&mut self, key_event: crate::native_terminal::KeyEvent, send_command: &mut dyn FnMut(String)) -> bool {
        match self.editor.handle_key(&key_event) {
            LineEditAction::Exit => return false,
            LineEditAction::Submit(command) => {
                send_command(command.clone());
                // Echo the sent command in the output area
                self.print_output(&format!("> {}\r\n", command));
            }
            LineEditAction::Updated => {
                self.draw_prompt();
            }
            LineEditAction::None => {}
        }
        true
    }
}

// Open a serial port with the given name and baud rate
fn open_serial_port(
    port_name: &str,
    baud_rate: u32,
) -> Result<Box<dyn SerialPort>, Box<dyn std::error::Error>> {
    let port = serial_new(port_name, baud_rate)
        .timeout(Duration::from_millis(50))
        .open()?;
    Ok(port)
}

/// Spawn the reader thread. Returns the receiver end of the channel.
fn spawn_reader_thread(
    read_port: Box<dyn SerialPort>,
    running: Arc<AtomicBool>,
    no_reconnect: bool,
    port_name: String,
    baud_rate: u32,
    write_rx: mpsc::Receiver<Vec<u8>>,
) -> mpsc::Receiver<ReaderEvent> {
    let (tx, rx) = mpsc::channel();

    thread::spawn(move || {
        let mut port = read_port;
        let mut buf = [0u8; SERIAL_READ_BUF_SIZE];
        let mut backoff_ms: u64 = 100;
        let current_port_name = port_name;

        while running.load(Ordering::SeqCst) {
            let mut write_error = false;
            loop {
                match write_rx.try_recv() {
                    Ok(data) => {
                        if port.write_all(&data).is_err() || port.flush().is_err() {
                            write_error = true;
                            break;
                        }
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        write_error = true;
                        break;
                    }
                }
            }

            if write_error {
                let _ = tx.send(ReaderEvent::Error("Serial port write error".into()));
                if no_reconnect {
                    break;
                }
                loop {
                    if !running.load(Ordering::SeqCst) {
                        return;
                    }
                    thread::sleep(Duration::from_millis(backoff_ms));
                    match open_serial_port(&current_port_name, baud_rate) {
                        Ok(new_port) => {
                            port = new_port;
                            let _ = tx.send(ReaderEvent::Reconnected);
                            backoff_ms = 100;
                            break;
                        }
                        Err(_) => {
                            backoff_ms = (backoff_ms * 2).min(2000);
                        }
                    }
                }
                continue;
            }

            match port.read(&mut buf) {
                Ok(n) if n > 0 => {
                    backoff_ms = 100;
                    if tx.send(ReaderEvent::Data(buf[..n].to_vec())).is_err() {
                        break;
                    }
                }
                Ok(_) => {}
                Err(ref e) if e.kind() == io::ErrorKind::TimedOut => {}
                Err(_e) => {
                    let _ = tx.send(ReaderEvent::Error("Serial port read error".into()));
                    if no_reconnect {
                        break;
                    }
                    loop {
                        if !running.load(Ordering::SeqCst) {
                            return;
                        }
                        thread::sleep(Duration::from_millis(backoff_ms));
                        match open_serial_port(&current_port_name, baud_rate) {
                            Ok(new_port) => {
                                port = new_port;
                                let _ = tx.send(ReaderEvent::Reconnected);
                                backoff_ms = 100;
                                break;
                            }
                            Err(_) => {
                                backoff_ms = (backoff_ms * 2).min(2000);
                            }
                        }
                    }
                }
            }
        }
    });

    rx
}

pub fn start_native(
    app_folder: String,
    serial_port_name: Option<String>,
    baud_rate: u32,
    no_reconnect: bool,
    log: bool,
    log_folder: String,
    vid: Option<String>,
    rx_timestamps: Option<String>,
    history_file_name: String,
) -> Result<(), Box<dyn std::error::Error>> {
    let rx_ts_mode = match rx_timestamps.as_deref() {
        Some("first") => Some(RxTimestampMode::First),
        Some("eol")   => Some(RxTimestampMode::Eol),
        Some(other) => {
            eprintln!("Warning: unknown --rx-timestamps value '{}', valid values are 'first' or 'eol'", other);
            None
        }
        None => None,
    };
    // Open log file if required
    let log_file = open_log_file(log, &log_folder)?;

    // Resolve port name
    let port_name = if let Some(name) = serial_port_name {
        name
    } else {
        let port_cmd = PortsCmd::new_with_vid(vid);
        match select_most_likely_port(&port_cmd, false) {
            Some(p) => p.port_name,
            None => {
                println!("Error: No suitable port found");
                std::process::exit(1);
            }
        }
    };

    // Open the serial port
    let port = open_serial_port(&port_name, baud_rate)?;

    // Running flag shared with reader thread
    let running = Arc::new(AtomicBool::new(true));

    // Spawn reader thread
    let (write_tx, write_rx) = mpsc::channel();

    let serial_rx = spawn_reader_thread(
        port,
        running.clone(),
        no_reconnect,
        port_name.clone(),
        baud_rate,
        write_rx,
    );

    // Set up display
    let history_file_path = format!("{}/{}", app_folder, history_file_name);
    let mut display = Display::new(&history_file_path, rx_ts_mode);
    display.init()?;
    display.draw_prompt();

    // Closure to send a command to the serial port
    let mut send_command = |command: String| {
        let mut data = command.into_bytes();
        data.push(b'\n');
        let _ = write_tx.send(data);
    };

    // Main loop
    while running.load(Ordering::SeqCst) {
        // 1. Drain ALL pending keyboard/resize events (non-blocking)
        while display.term.poll_event(Duration::ZERO) {
            match display.term.read_event() {
                Some(TermEvent::Key(ke)) => {
                    if !display.handle_key_event(ke, &mut send_command) {
                        running.store(false, Ordering::SeqCst);
                        break;
                    }
                }
                Some(TermEvent::Resize(cols, rows)) => {
                    display.handle_resize(cols, rows);
                }
                None => break,
            }
        }

        if !running.load(Ordering::SeqCst) {
            break;
        }

        // 2. Drain pending serial data (non-blocking, bounded so we re-check keys).
        // draw_prompt is called once after the full drain rather than per-message
        // to avoid redundant ANSI escape sequences at high data rates.
        const MAX_SERIAL_DRAIN: usize = 64;
        let mut had_serial_data = false;
        let mut drain_error = false;
        for _ in 0..MAX_SERIAL_DRAIN {
            match serial_rx.try_recv() {
                Ok(ReaderEvent::Data(bytes)) => {
                    let text = String::from_utf8_lossy(&bytes);
                    display.print_output(&text);
                    write_to_log(&log_file, &text);
                    had_serial_data = true;
                }
                Ok(ReaderEvent::Error(msg)) => {
                    display.show_error(&msg);
                    drain_error = true;
                    break;
                }
                Ok(ReaderEvent::Reconnected) => {
                    display.show_info("Reconnected");
                    thread::sleep(Duration::from_millis(500));
                    display.draw_prompt();
                    had_serial_data = false; // prompt already drawn
                    break;
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    display.show_error("Serial reader thread disconnected");
                    running.store(false, Ordering::SeqCst);
                    drain_error = true;
                    break;
                }
            }
        }
        // Redraw the prompt exactly once after the drain batch
        if had_serial_data && !drain_error {
            display.draw_prompt();
        }

        // 3. Wait briefly for next event (avoids busy-spin).
        // 5ms keeps latency low while still yielding the CPU.
        let _ = display.term.poll_event(Duration::from_millis(5));
    }

    // Clean up — Display's Drop will restore terminal
    display.term.cleanup();
    println!("Exiting...\r");
    Ok(())
}

pub fn start_non_native(
    app_folder: String,
    port: Option<String>,
    baud: u32,
    no_reconnect: bool,
    log: bool,
    log_folder: String,
    vid: Option<String>,
    rx_timestamps: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut args = vec![
        "monitor".to_string(),
        app_folder.clone(),
        "-b".to_string(),
        baud.to_string(),
    ];
    if let Some(p) = port {
        args.push("-p".to_string());
        args.push(p);
    }
    if let Some(v) = vid {
        args.push("-v".to_string());
        args.push(v);
    }
    if no_reconnect {
        args.push("-n".to_string());
    }
    if log {
        args.push("-l".to_string());
        args.push("-g".to_string());
        args.push(log_folder);
    }
    if let Some(mode) = rx_timestamps {
        args.push("--rx-timestamps".to_string());
        args.push(mode);
    }

    let process = Command::new("raft.exe")
        .args(args)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn();

    match process {
        Ok(mut child) => {
            match child.wait() {
                Ok(_status) => {}
                Err(e) => {
                    println!("Error in serial monitor: {:?}", e);
                }
            }
        }
        Err(e) => {
            println!("Error starting serial monitor: {:?}", e);
        }
    }

    Ok(())
}
