// RaftCLI: Serial Monitor V2
// Rob Dobson 2024-2026
//
// Simplified two-thread architecture:
//   - Reader thread: owns the read-half of the serial port, sends data via mpsc channel
//   - Main thread: owns the terminal + write-half of the serial port, no Mutex needed

use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    style::{Color, ResetColor, SetForegroundColor},
    terminal::{self, ClearType},
    execute,
};
use serialport_fix_stop_bits::{new as serial_new, SerialPort};
use std::io::{self, Write};
use std::process::{Command, Stdio};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};
use std::thread;
use std::time::Duration;

use crate::app_ports::{select_most_likely_port, PortsCmd};
use crate::cmd_history::CommandHistory;
use crate::console_log::{open_log_file, write_to_log};

// Size of the serial read buffer — large enough to handle high baud rates
const SERIAL_READ_BUF_SIZE: usize = 4096;

// Events sent from the reader thread to the main thread
enum ReaderEvent {
    Data(Vec<u8>),
    Error(String),
    Reconnected,
}

// Display state — replaces TerminalIO with self-tracked cursor and resize support
struct Display {
    cols: u16,
    rows: u16,
    output_col: u16,
    output_row: u16,
    command_buffer: String,
    is_error: bool,
    command_history: CommandHistory,
}

impl Display {
    fn new(history_file_path: &str) -> Display {
        Display {
            cols: 80,
            rows: 24,
            output_col: 0,
            output_row: 0,
            command_buffer: String::new(),
            is_error: false,
            command_history: CommandHistory::new(history_file_path),
        }
    }

    fn init(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let (cols, rows) = terminal::size()?;
        self.cols = cols;
        self.rows = rows;

        terminal::enable_raw_mode()?;
        execute!(
            io::stdout(),
            terminal::Clear(ClearType::All),
            cursor::MoveTo(0, 0)
        )?;
        Ok(())
    }

    fn handle_resize(&mut self, cols: u16, rows: u16) {
        self.cols = cols;
        self.rows = rows;
        // Clamp output cursor to new bounds
        if self.output_row >= rows.saturating_sub(1) {
            self.output_row = rows.saturating_sub(2);
        }
        // Redraw prompt on the new last row
        self.draw_prompt();
    }

    /// Write serial output data to the scrollable area (all rows except the last).
    /// Cursor position is self-tracked — no cursor::position() query needed.
    fn print_output(&mut self, data: &str) {
        self.is_error = false;

        // Clear the prompt line so output doesn't collide with it
        execute!(
            io::stdout(),
            cursor::MoveTo(0, self.rows - 1),
            terminal::Clear(ClearType::CurrentLine)
        )
        .unwrap();

        // Move to saved output position
        execute!(
            io::stdout(),
            cursor::MoveTo(self.output_col, self.output_row)
        )
        .unwrap();

        // The last row available for output (one above the prompt)
        let max_output_row = self.rows.saturating_sub(2);

        let stdout = io::stdout();
        let mut handle = stdout.lock();

        for ch in data.chars() {
            match ch {
                '\n' => {
                    self.output_col = 0;
                    if self.output_row >= max_output_row {
                        // At the bottom of the output area — scroll up to make room
                        execute!(handle, terminal::ScrollUp(1)).unwrap();
                        execute!(handle, cursor::MoveTo(0, self.output_row)).unwrap();
                    } else {
                        self.output_row += 1;
                        execute!(handle, cursor::MoveTo(self.output_col, self.output_row)).unwrap();
                    }
                }
                '\r' => {
                    self.output_col = 0;
                    execute!(handle, cursor::MoveTo(self.output_col, self.output_row)).unwrap();
                }
                c if !c.is_control() => {
                    write!(handle, "{}", c).unwrap();
                    self.output_col += 1;
                    if self.output_col >= self.cols {
                        // Line wrapped
                        self.output_col = 0;
                        if self.output_row >= max_output_row {
                            execute!(handle, terminal::ScrollUp(1)).unwrap();
                            execute!(handle, cursor::MoveTo(0, self.output_row)).unwrap();
                        } else {
                            self.output_row += 1;
                        }
                    }
                }
                _ => {
                    // Skip other control characters
                }
            }
        }

        handle.flush().unwrap();
        drop(handle);

        self.draw_prompt();
    }

    fn draw_prompt(&self) {
        execute!(
            io::stdout(),
            cursor::MoveTo(0, self.rows - 1),
            terminal::Clear(ClearType::CurrentLine),
            SetForegroundColor(Color::Yellow),
        )
        .unwrap();

        print!("> {}", self.command_buffer);

        execute!(io::stdout(), ResetColor).unwrap();
        io::stdout().flush().unwrap();
    }

    fn show_error(&mut self, msg: &str) {
        execute!(
            io::stdout(),
            cursor::MoveTo(0, self.rows - 1),
            terminal::Clear(ClearType::CurrentLine),
            SetForegroundColor(Color::Red),
        )
        .unwrap();
        print!("! {}", msg);
        execute!(io::stdout(), ResetColor).unwrap();
        io::stdout().flush().unwrap();
        self.is_error = true;
    }

    fn show_info(&mut self, msg: &str) {
        execute!(
            io::stdout(),
            cursor::MoveTo(0, self.rows - 1),
            terminal::Clear(ClearType::CurrentLine),
            SetForegroundColor(Color::Green),
        )
        .unwrap();
        print!("> {}", msg);
        execute!(io::stdout(), ResetColor).unwrap();
        io::stdout().flush().unwrap();
    }

    /// Handle a key event. Returns false if the monitor should exit.
    fn handle_key_event(&mut self, key_event: KeyEvent, send_command: &mut dyn FnMut(String)) -> bool {
        match key_event.code {
            KeyCode::Char('c') | KeyCode::Char('x')
                if key_event.modifiers == KeyModifiers::CONTROL =>
            {
                return false;
            }
            KeyCode::Esc => return false,
            KeyCode::Enter => {
                let command = self.command_buffer.clone();
                send_command(command.clone());
                self.command_history.add_command(&command);
                self.command_buffer.clear();
                // Echo the sent command in the output area
                self.print_output(&format!("> {}\r\n", command));
            }
            KeyCode::Backspace => {
                if !self.command_buffer.is_empty() {
                    self.command_buffer.pop();
                    self.draw_prompt();
                }
            }
            KeyCode::Char(c) => {
                self.command_buffer.push(c);
                self.draw_prompt();
            }
            KeyCode::Up => {
                self.command_history.move_up();
                self.command_buffer = self.command_history.get_current();
                self.draw_prompt();
            }
            KeyCode::Down => {
                self.command_history.move_down();
                self.command_buffer = self.command_history.get_current();
                self.draw_prompt();
            }
            _ => {}
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
/// The reader thread exclusively owns `read_port` — no Mutex needed.
fn spawn_reader_thread(
    read_port: Box<dyn SerialPort>,
    running: Arc<AtomicBool>,
    no_reconnect: bool,
    port_name: String,
    baud_rate: u32,
) -> mpsc::Receiver<ReaderEvent> {
    let (tx, rx) = mpsc::channel();

    thread::spawn(move || {
        let mut port = read_port;
        let mut buf = [0u8; SERIAL_READ_BUF_SIZE];
        let mut backoff_ms: u64 = 100;

        while running.load(Ordering::SeqCst) {
            match port.read(&mut buf) {
                Ok(n) if n > 0 => {
                    backoff_ms = 100; // reset backoff on success
                    // Send only the bytes that were read
                    if tx.send(ReaderEvent::Data(buf[..n].to_vec())).is_err() {
                        break; // main thread dropped the receiver
                    }
                }
                Ok(_) => {
                    // Zero bytes — just loop
                }
                Err(ref e) if e.kind() == io::ErrorKind::TimedOut => {
                    // Normal timeout from the 50ms read timeout — loop immediately
                }
                Err(_e) => {
                    let _ = tx.send(ReaderEvent::Error("Serial port read error".into()));
                    if no_reconnect {
                        break;
                    }
                    // Reconnection loop with backoff
                    loop {
                        if !running.load(Ordering::SeqCst) {
                            return;
                        }
                        thread::sleep(Duration::from_millis(backoff_ms));
                        match open_serial_port(&port_name, baud_rate) {
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
            // No sleep here — the 50ms read timeout on the port provides the pacing
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
    history_file_name: String,
) -> Result<(), Box<dyn std::error::Error>> {
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

    // Clone the port for writing — main thread keeps write_port, reader thread gets port
    let write_port: Option<Box<dyn SerialPort>> = match port.try_clone() {
        Ok(cloned) => {
            // We have a clone — we'll give the original to the reader thread
            // and keep the clone for writing on the main thread
            Some(cloned)
        }
        Err(_) => None,
    };

    // Running flag shared with reader thread
    let running = Arc::new(AtomicBool::new(true));

    // Spawn reader thread — it owns `port` exclusively
    let serial_rx = spawn_reader_thread(
        port,
        running.clone(),
        no_reconnect,
        port_name.clone(),
        baud_rate,
    );

    // The write port lives on the main thread — no Mutex, no extra thread
    let mut write_port = write_port;

    // Set up display
    let history_file_path = format!("{}/{}", app_folder, history_file_name);
    let mut display = Display::new(&history_file_path);
    display.init()?;

    // Show initial prompt
    display.draw_prompt();

    // Closure to send a command to the serial port
    let mut send_command = |command: String| {
        if let Some(ref mut wp) = write_port {
            let _ = wp.write(command.as_bytes());
            let _ = wp.write(b"\n");
        }
    };

    // Main loop
    while running.load(Ordering::SeqCst) {
        // 1. Drain ALL pending keyboard/resize events (non-blocking)
        while crossterm::event::poll(Duration::ZERO)? {
            match event::read()? {
                Event::Key(ke) if ke.kind == KeyEventKind::Press => {
                    if !display.handle_key_event(ke, &mut send_command) {
                        running.store(false, Ordering::SeqCst);
                        break;
                    }
                }
                Event::Resize(cols, rows) => {
                    display.handle_resize(cols, rows);
                }
                _ => {}
            }
        }

        if !running.load(Ordering::SeqCst) {
            break;
        }

        // 2. Drain all pending serial data (non-blocking)
        loop {
            match serial_rx.try_recv() {
                Ok(ReaderEvent::Data(bytes)) => {
                    let text = String::from_utf8_lossy(&bytes);
                    display.print_output(&text);
                    write_to_log(&log_file, &text);
                }
                Ok(ReaderEvent::Error(msg)) => {
                    display.show_error(&msg);
                }
                Ok(ReaderEvent::Reconnected) => {
                    display.show_info("Reconnected");
                    // Brief pause to show the message, then restore prompt
                    thread::sleep(Duration::from_millis(500));
                    display.draw_prompt();
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    display.show_error("Serial reader thread disconnected");
                    running.store(false, Ordering::SeqCst);
                    break;
                }
            }
        }

        // 3. Wait briefly for next event (avoids busy-spin)
        let _ = crossterm::event::poll(Duration::from_millis(15));
    }

    // Clean up
    terminal::disable_raw_mode()?;
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
    args.push("--v2".to_string());

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
