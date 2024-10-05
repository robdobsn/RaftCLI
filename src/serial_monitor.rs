// RaftCLI: Serial monitor module
// Rob Dobson 2024

use crossterm::{
    cursor, event::{self, Event, KeyCode, KeyEventKind, KeyModifiers}, execute, style::{Color, ResetColor, SetForegroundColor}, terminal,
};
use serialport_fix_stop_bits::{new, SerialPort};
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc, Mutex,
};
use std::thread;
use std::time::Duration;

use crate::{app_ports::{select_most_likely_port, PortsCmd}, cmd_history::CommandHistory};

struct LogFileInfo {
    file: std::fs::File,
    last_write: std::time::Instant,
}
type SharedLogFile = Arc<Mutex<Option<LogFileInfo>>>;

struct TerminalOut {
    command_buffer: String,
    cursor_col: u16,
    cursor_row: u16,
    cols: u16,
    rows: u16,
    is_error: bool,
}

impl TerminalOut {
    fn new() -> TerminalOut {
        TerminalOut {
            command_buffer: String::new(),
            cursor_col: 0,
            cursor_row: 0,
            cols: 0,
            rows: 0,
            is_error: false,
        }
    }

    fn init(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let (_cols, rows) = terminal::size()?;
        self.cols = _cols;
        self.rows = rows;
        // Setup terminal for raw mode
        terminal::enable_raw_mode()?;
        execute!(
            std::io::stdout(),
            terminal::Clear(terminal::ClearType::All),
            cursor::MoveTo(0, 0)
        )?;
        Ok(())
    }

    fn print(&mut self, data: &str, force_show: bool) {

        if !force_show && self.is_error {
            return;
        }

        // Clear error flag
        self.is_error = false;

        // Clear the last line of the terminal (command buffer)
        execute!(
            std::io::stdout(),
            cursor::MoveTo(0, self.rows - 1),
            terminal::Clear(terminal::ClearType::CurrentLine)
        ).unwrap();

        // Move the cursor to the position of the last output
        execute!(
            std::io::stdout(),
            cursor::MoveTo(self.cursor_col, self.cursor_row)
        ).unwrap();

        // Display the received data
        self.display_serial_data(&data);

        // Get the cursor position
        let (cursor_col, mut cursor_row) = cursor::position().unwrap();

        // If the cursor is not at the first column then add a newline
        if cursor_col != 0 && cursor_row == self.rows - 1 {
            print!("\n");
            cursor_row -= 1;
        }

        // Save the cursor position
        self.cursor_col = cursor_col;
        self.cursor_row = cursor_row;

        // Move the cursor to the bottom line and clear it
        execute!(
            std::io::stdout(),
            cursor::MoveTo(0, self.rows - 1),
            terminal::Clear(terminal::ClearType::CurrentLine),
            SetForegroundColor(Color::Yellow),
        ).unwrap();

        // Display the command buffer
        print!("> {}", self.command_buffer);

        // Reset the text color
        execute!(std::io::stdout(), ResetColor).unwrap();

        // Flush the output
        std::io::stdout().flush().unwrap();
    }

    fn show_error(&mut self, error_msg: &str) {

        // Move the cursor to the bottom line and clear it
        execute!(
            std::io::stdout(),
            cursor::MoveTo(0, self.rows - 1),
            terminal::Clear(terminal::ClearType::CurrentLine),
            SetForegroundColor(Color::Red),
        ).unwrap();

        // Display the error message
        print!("! {}", error_msg);

        // Reset the text color
        execute!(std::io::stdout(), ResetColor).unwrap();

        // Flush the output
        std::io::stdout().flush().unwrap();

        // Set the error flag
        self.is_error = true;
    }

    fn display_serial_data(&mut self, data: &str) {
        print!("{}", data);
        std::io::stdout().flush().unwrap();
    }

    fn get_command_buffer(&self) -> String {
        self.command_buffer.clone()
    }

    fn clear_command_buffer(&mut self) {
        self.command_buffer.clear();
        self.print("", false);
    }

    fn add_to_command_buffer(&mut self, c: char) {
        self.command_buffer.push(c);
        self.print("", false);
    }

    fn add_str_to_command_buffer(&mut self, s: &str) {
        self.command_buffer.push_str(s);
        self.print("", true);
    }

    fn backspace_command_buffer(&mut self) {
        if self.command_buffer.len() > 0 {
            self.command_buffer.pop();
            self.print("", false);
        }
    }
}

// Logging to file
fn open_log_file(log_to_file: bool, log_folder: String) -> Result<SharedLogFile, std::io::Error> {
    if log_to_file && log_folder.len() > 0 && log_folder != "none" {
        // Create a log file
        let name = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
        let log_file_name = format!("{}/{}.log", log_folder, name);
        std::fs::create_dir_all(&log_folder)?;
        // Open the log file
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_file_name)?;
        return Ok(Arc::new(Mutex::new(Some(LogFileInfo {
            file,
            last_write: std::time::Instant::now(),
        }))));
    }
    Ok(Arc::new(Mutex::new(None)))
}

struct CommandAndTime {
    user_input: String,
    _time: std::time::Instant,
}

pub fn start_native(
    app_folder: String,
    port: Option<String>,
    baud_rate: u32,
    no_reconnect: bool,
    log: bool,
    log_folder: String,
    vid: Option<String>
) -> Result<(), Box<dyn std::error::Error>> {

    // Command history in the app folder
    let mut history_file_path = std::path::PathBuf::from(&app_folder);
    history_file_path.push("raftcli_history.txt");
    let history_file_path_str = history_file_path.to_str().unwrap().to_string();
    let command_history = Arc::new(Mutex::new(CommandHistory::new(&history_file_path_str)));

    // Open log file if required
    let log_file = if log {
        let file = open_log_file(log, log_folder)?;
        file
    } else {
        Arc::new(Mutex::new(None))
    };

    // Arc and AtomicBool for controlling the running state
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();

    // Channels for communication between the serial thread and the main thread
    let (serial_read_tx, serial_read_rx) = mpsc::channel();
    let (serial_write_tx, serial_write_rx) = mpsc::channel::<CommandAndTime>();

    // Extract port and baud rate arguments
    let port = if let Some(port) = port {
        port
    } else {
        // Use select_most_likely_port if no specific port is provided
        let port_cmd = PortsCmd::new_with_vid(vid);
        match select_most_likely_port(&port_cmd, false) {
            Some(p) => p.port_name,
            None => {
                println!("Error: No suitable port found");
                std::process::exit(1);
            }
        }
    };
    
    // Function to open the serial port
    fn open_serial_port(
        port: &str,
        baud_rate: u32,
    ) -> Result<Box<dyn SerialPort>, Box<dyn std::error::Error>> {
        let port = new(port, baud_rate)
            .timeout(Duration::from_millis(100))
            .open()?;
        Ok(port)
    }

    // Open the serial port and wrap it in an Arc<Mutex<>>
    let serial_port = Arc::new(Mutex::new(open_serial_port(&port, baud_rate)?));

    // Clone the Arc for the serial communication thread
    let serial_port_clone = Arc::clone(&serial_port);

    // Terminal output
    let terminal_out = Arc::new(Mutex::new(TerminalOut::new()));
    terminal_out.lock().unwrap().init().unwrap();

    // Clone the Arc for the terminal output
    let terminal_out_clone = Arc::clone(&terminal_out);

    // Spawn a thread to handle reading from the serial port
    thread::spawn(move || {
        while r.load(Ordering::SeqCst) {
            let mut buffer: Vec<u8> = vec![0; 100];
            let result = {
                let mut serial_port_lock = serial_port_clone.lock().unwrap();
                serial_port_lock.read(&mut buffer)
            };
            match result {
                Ok(n) if n > 0 => {
                    let received = String::from_utf8_lossy(&buffer[..n]);
                    serial_read_tx.send(received.to_string())
                        .expect("Failed to send data to main thread");
                    if let Ok(mut log_file) = log_file.lock() {
                        if let Some(log_file_info) = log_file.as_mut() {
                            write!(log_file_info.file, "{}", received).unwrap();
                            log_file_info.last_write = std::time::Instant::now();
                        }
                    }
                }
                Ok(_) => {}
                Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => {}
                Err(_e) => {
                    terminal_out_clone.lock().unwrap().show_error("Serial port read error");
                    if no_reconnect {
                        break;
                    }
                    terminal_out_clone.lock().unwrap().show_error("Serial port attempting to reconnect...");
                    thread::sleep(Duration::from_millis(50));
                    match open_serial_port(&port, baud_rate) {
                        Ok(new_port) => {
                            *serial_port_clone.lock().unwrap() = new_port;
                        }
                        Err(_e) => {
                            // eprintln!("Serial port reconnection failed: {:?}\r", e);
                        }
                    }
                }
            }

            // Sleep the thread to allow terminal input
            thread::sleep(Duration::from_millis(1));
        }
        // eprintln!("Serial monitor exiting...\r");
    });

    // Spawn a thread to handle writing to the serial port
    let serial_port_clone = Arc::clone(&serial_port);
    thread::spawn(move || {
        while let Ok(command) = serial_write_rx.recv() {
            // println!("Time to receive command: {:?}", command.time.elapsed());
            let mut serial_port_lock = serial_port_clone.lock().unwrap();
            // println!("Time to lock port: {:?}", command.time.elapsed());
            let _ = serial_port_lock.write(command.user_input.as_bytes());
            let _ = serial_port_lock.write(&[b'\n']);
            // println!("Time to write command: {:?}", command.time.elapsed());
        }
    });

    // Print nothing to display the command prompt
    terminal_out.lock().unwrap().print("", false);

    // Main loop to handle terminal events and print received serial data
    while running.load(Ordering::SeqCst) {
        // Handle serial data
        if let Ok(received) = serial_read_rx.try_recv() {
            terminal_out.lock().unwrap().print(&received, true);
        }

        // Handle keyboard input
        if event::poll(Duration::from_millis(0))? {
            if let Event::Key(key_event) = event::read()? {
                if key_event.kind == KeyEventKind::Press {
                    match key_event.code {
                        KeyCode::Char(c)
                            if key_event.modifiers == KeyModifiers::CONTROL
                                && (c == 'c' || c == 'x') =>
                        {
                            running.store(false, Ordering::SeqCst);
                        }
                        KeyCode::Esc => {
                            running.store(false, Ordering::SeqCst);
                        }
                        KeyCode::Enter => {
                            // print!("⏎");
                            let key_detect_time = std::time::Instant::now();
                            let user_input = terminal_out.lock().unwrap().get_command_buffer();
                            let command: CommandAndTime = CommandAndTime {
                                user_input: user_input.clone(),
                                _time: key_detect_time
                            };
                            // println!("Time to get command buffer: {:?}", key_detect_time.elapsed());
                            serial_write_tx.send(command).expect("Failed to send command to write thread");
                            // Add the command to history
                            command_history.lock().unwrap().add_command(&user_input);
                            // println!("Time to send command: {:?}", key_detect_time.elapsed());
                            terminal_out.lock().unwrap().clear_command_buffer();
                        }
                        KeyCode::Backspace => {
                            terminal_out.lock().unwrap().backspace_command_buffer();
                        }
                        KeyCode::Char(c) => {
                            terminal_out.lock().unwrap().add_to_command_buffer(c);
                        }
                        KeyCode::Up => {
                            if let Some(previous_command) = command_history.lock().unwrap().get_previous() {
                                terminal_out.lock().unwrap().clear_command_buffer();
                                terminal_out.lock().unwrap().add_str_to_command_buffer(previous_command);
                            }
                        }
                        KeyCode::Down => {
                            if let Some(next_command) = command_history.lock().unwrap().get_next() {
                                terminal_out.lock().unwrap().clear_command_buffer();
                                terminal_out.lock().unwrap().add_str_to_command_buffer(next_command);
                            } else {
                                terminal_out.lock().unwrap().clear_command_buffer();
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
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
    vid: Option<String>
) -> Result<(), Box<dyn std::error::Error>> {
    // Setup args
    let mut args = vec![
        "monitor".to_string(),
        app_folder.clone(),
        "-b".to_string(),
        baud.to_string(),
    ];
    if port.is_some() {
        args.push("-p".to_string());
        args.push(port.unwrap());
    }
    if vid.is_some() {
        args.push("-v".to_string());
        args.push(vid.unwrap());
    }
    if no_reconnect {
        args.push("-n".to_string());
    }
    if log {
        args.push("-l".to_string());
        args.push("-g".to_string());
        args.push(log_folder);
    }

    // Run the serial monitor
    let process = Command::new("raft.exe")
        .args(args)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn();

    // Check for error
    match process {
        Ok(mut child) => {
            // Wait for the process to complete
            match child.wait() {
                Ok(_status) => {
                    // println!("Process exited with status: {}", _status)
                }
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
