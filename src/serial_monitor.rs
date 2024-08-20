// RaftCLI: Serial monitor module
// Rob Dobson 2024

use crossterm::{
    cursor, event::{self, Event, KeyCode, KeyEventKind, KeyModifiers}, execute, style::{Color, ResetColor, SetForegroundColor}, terminal::{self, ScrollDown, ScrollUp}
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

use crate::app_ports::{select_most_likely_port, PortsCmd};

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
}

impl TerminalOut {
    fn new() -> TerminalOut {
        TerminalOut {
            command_buffer: String::new(),
            cursor_col: 0,
            cursor_row: 0,
            cols: 0,
            rows: 0,
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

    fn print(&mut self, data: &str) {

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

        // Save the cursor position
        let (cursor_col, cursor_row) = cursor::position().unwrap();
        self.cursor_col = cursor_col;
        self.cursor_row = cursor_row;

        // If the cursor is not at the first column then add a newline
        if cursor_col != 0 {
            print!("\n");
            self.cursor_row -= 1;
        }

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

    fn display_serial_data(&mut self, data: &str) {
        print!("{}", data);
        // for byte in data.bytes() {
        //     if (byte as char) >= ' ' && (byte as char) <= '~' {
        //         print!("{}", byte as char);
        //     } else if byte == 0x0A {
        //         print!("\n");
        //     } else {
        //         print!(".");
        //     }
        // }
        std::io::stdout().flush().unwrap();
    }

    fn get_command_buffer(&self) -> String {
        self.command_buffer.clone()
    }

    fn clear_command_buffer(&mut self) {
        self.command_buffer.clear();
        self.print("");
    }

    fn add_to_command_buffer(&mut self, c: char) {
        self.command_buffer.push(c);
        self.print("");
    }

    fn backspace_command_buffer(&mut self) {
        if self.command_buffer.len() > 0 {
            self.command_buffer.pop();
            self.print("");
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

// fn display_serial_data(data: &str, _rows: u16) {
//     // // Move existing text up
//     // execute!(
//     //     std::io::stdout(),
//     //     cursor::MoveTo(0, rows - 1),
//     //     terminal::Clear(terminal::ClearType::CurrentLine),
//     // )?;
//     // print!("{}", received);
//     // // Redraw the command buffer
//     // display_cmd_buffer(&command_buffer, rows);

//     for byte in data.bytes() {
//         if (byte as char) >= ' ' && (byte as char) <= '~' {
//             print!("{}", byte as char);
//         } else if byte == 0x0A {
//             print!("\n");
//         } else {
//             print!(".");
//         }
//     }

//     // let mut lines = data.lines();
//     // let mut line = lines.next();
//     // while let Some(l) = line {
//     //     execute!(
//     //         std::io::stdout(),
//     //         cursor::MoveTo(0, rows - 1),
//     //         terminal::Clear(terminal::ClearType::CurrentLine),
//     //     )
//     //     .unwrap();
//     //     print!("{}", l);
//     //     std::io::stdout().flush().unwrap();
//     //     line = lines.next();
//     // }
// }

// fn remove_cmd_buffer(_command_buffer: &str, rows: u16) {
//     let mut stdout = std::io::stdout();

//     // Move the cursor to the top-left corner of the terminal
//     execute!(stdout, cursor::MoveTo(0, 0)).unwrap();

//     // Scroll the terminal content up by one line (effectively moving all text down by one line)
//     execute!(stdout, ScrollUp(1)).unwrap();

//     // Move the cursor to the bottom line
//     execute!(
//         std::io::stdout(),
//         cursor::MoveTo(0, rows - 1),
//         terminal::Clear(terminal::ClearType::CurrentLine)
//     )
//     .unwrap();
// }

// fn display_cmd_buffer(command_buffer: &str, rows: u16) {
//     // Move existing text up one line
//     let mut stdout = std::io::stdout();

//     // Scroll the terminal content down by one line, which moves all text up by one line
//     execute!(stdout, ScrollDown(1)).unwrap();

//     // Flush to apply the changes
//     stdout.flush().unwrap();

//     // Move the cursor to the bottom line
//     execute!(
//         std::io::stdout(),
//         cursor::MoveTo(0, rows - 1),
//         terminal::Clear(terminal::ClearType::CurrentLine),
//         SetForegroundColor(Color::Yellow),
//     )
//     .unwrap();
//     print!("> {}", command_buffer);
//     execute!(std::io::stdout(), ResetColor).unwrap();
//     std::io::stdout().flush().unwrap();
// }

pub fn start_native(
    port: Option<String>,
    baud_rate: u32,
    no_reconnect: bool,
    log: bool,
    log_folder: String,
    vid: Option<String>
) -> Result<(), Box<dyn std::error::Error>> {
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

    // Channel for communication between the serial thread and the main thread
    let (tx, rx) = mpsc::channel();

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

    // Spawn a thread to handle serial port communication
    thread::spawn(move || {
        while r.load(Ordering::SeqCst) {
            let mut buffer: Vec<u8> = vec![0; 10000];
            let mut serial_port_lock = serial_port_clone.lock().unwrap();
            match serial_port_lock.read(&mut buffer) {
                Ok(n) if n > 0 => {
                    let received = String::from_utf8_lossy(&buffer[..n]);
                    tx.send(received.to_string())
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
                Err(e) => {
                    eprintln!("Serial port read error: {:?}\r", e);
                    if no_reconnect {
                        break;
                    }
                    eprintln!("Serial port attempting to reconnect...\r");
                    drop(serial_port_lock); // Unlock the mutex before attempting to reconnect
                    thread::sleep(Duration::from_secs(1));
                    match open_serial_port(&port, baud_rate) {
                        Ok(new_port) => {
                            *serial_port_clone.lock().unwrap() = new_port;
                        }
                        Err(e) => {
                            eprintln!("Serial port reconnection failed: {:?}\r", e);
                        }
                    }
                }
            }
        }
        eprintln!("Serial monitor exiting...\r");
    });

    // Terminal output
    let mut terminal_out = TerminalOut::new();
    terminal_out.init()?;

    // // Setup terminal for raw mode
    // terminal::enable_raw_mode()?;
    // execute!(
    //     std::io::stdout(),
    //     terminal::Clear(terminal::ClearType::All),
    //     cursor::MoveTo(0, 0)
    // )?;

    // let mut command_buffer = String::new();
    // let (_cols, rows) = terminal::size()?;

    // // Initially display the command buffer
    // display_cmd_buffer(&command_buffer, rows);

    // Main loop to handle terminal events and print received serial data
    while running.load(Ordering::SeqCst) {
        // Handle serial data
        while let Ok(received) = rx.try_recv() {
            terminal_out.print(&received);
            // remove_cmd_buffer(&command_buffer, rows);
            // display_serial_data(&received, rows);
            // display_cmd_buffer(&command_buffer, rows)
        }

        // Handle keyboard input
        if event::poll(Duration::from_millis(1))? {
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
                            let mut serial_port_lock = serial_port.lock().unwrap();
                            let _ = serial_port_lock.write(terminal_out.get_command_buffer().as_bytes());
                            let _ = serial_port_lock.write(&[b'\n']);
                            terminal_out.clear_command_buffer();
                        }
                        KeyCode::Backspace => {
                            terminal_out.backspace_command_buffer();
                        }
                        KeyCode::Char(c) => {
                            terminal_out.add_to_command_buffer(c);
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

