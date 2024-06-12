// RaftCLI: Serial monitor module
// Rob Dobson 2024

use std::sync::{atomic::{AtomicBool, Ordering}, mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;
use crossterm::{event::{self, Event, KeyCode, KeyModifiers}, terminal, execute, cursor};
use serialport::{SerialPort, new};
use std::io::Write;
use std::process::{Command, Stdio};

struct LogFileInfo {
    file: std::fs::File,
    last_write: std::time::Instant,
}
type SharedLogFile = Arc<Mutex<Option<LogFileInfo>>>;

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

pub fn start_native(
    port_name: String,
    baud_rate: u32,
    no_reconnect: bool,
    log: bool,
    log_folder: String,
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

    // Function to open the serial port
    fn open_serial_port(
        port_name: &str,
        baud_rate: u32,
    ) -> Result<Box<dyn SerialPort>, Box<dyn std::error::Error>> {
        let port = new(port_name, baud_rate)
            .timeout(Duration::from_millis(10))
            .open()?;
        Ok(port)
    }

    // Open the serial port and wrap it in an Arc<Mutex<>>
    let serial_port = Arc::new(Mutex::new(open_serial_port(&port_name, baud_rate)?));

    // Clone the Arc for the serial communication thread
    let serial_port_clone = Arc::clone(&serial_port);

    // Spawn a thread to handle serial port communication
    thread::spawn(move || {
        while r.load(Ordering::SeqCst) {
            let mut buffer: Vec<u8> = vec![0; 1024];
            let mut serial_port_lock = serial_port_clone.lock().unwrap();
            match serial_port_lock.read(&mut buffer) {
                Ok(n) if n > 0 => {
                    let received = String::from_utf8_lossy(&buffer[..n]);
                    tx.send(received.to_string()).expect("Failed to send data to main thread");
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
                    eprintln!("Attempting to reconnect...\r");
                    drop(serial_port_lock); // Unlock the mutex before attempting to reconnect
                    thread::sleep(Duration::from_secs(1));
                    match open_serial_port(&port_name, baud_rate) {
                        Ok(new_port) => {
                            *serial_port_clone.lock().unwrap() = new_port;
                        }
                        Err(e) => {
                            eprintln!("Reconnection failed: {:?}\r", e);
                        }
                    }
                }
            }
        }
        eprintln!("Serial thread exiting...\r");
    });

    // Setup terminal for raw mode
    terminal::enable_raw_mode()?;
    execute!(
        std::io::stdout(),
        terminal::Clear(terminal::ClearType::All),
        cursor::MoveTo(0, 0)
    )?;

    // Main loop to handle terminal events and print received serial data
    while running.load(Ordering::SeqCst) {
        // Handle serial data
        while let Ok(received) = rx.try_recv() {
            print!("{}", received);
            std::io::stdout().flush().unwrap();
        }

        // Handle keyboard input
        if event::poll(Duration::from_millis(1))? {
            if let Event::Key(key_event) = event::read()? {
                match key_event.code {
                    KeyCode::Char(c) if key_event.modifiers == KeyModifiers::CONTROL && (c == 'c' || c == 'x') => {
                        running.store(false, Ordering::SeqCst);
                    }
                    KeyCode::Esc => {
                        running.store(false, Ordering::SeqCst);
                    }
                    KeyCode::Enter => {
                        let _ = serial_port.lock().unwrap().write(&[b'\n']);
                        std::io::stdout().flush().unwrap();
                    }
                    KeyCode::Backspace => {
                        let _ = serial_port.lock().unwrap().write(&[b'\x08']);
                        print!("\x08 \x08");
                        std::io::stdout().flush().unwrap();
                    }
                    KeyCode::Char(c) => {
                        let _ = serial_port.lock().unwrap().write(&[c as u8]);
                        print!("{}", c);
                        std::io::stdout().flush().unwrap();
                    }
                    _ => {}
                }
            }
        }
    }

    // Clean up
    terminal::disable_raw_mode()?;
    println!("Exiting...\r");

    Ok(())
}

pub fn start_non_native(port: String, baud: u32, no_reconnect: bool,
    log: bool, log_folder: String) -> Result<(), Box<dyn std::error::Error>> {

// Setup args
let mut args = vec!["monitor".to_string(), "-p".to_string(), port, "-b".to_string(), baud.to_string()];
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
       },
   Err(e) => { 
       println!("Error in serial monitor: {:?}", e);
   },
}
},
Err(e) => {
println!("Error starting serial monitor: {:?}", e);
},
}

Ok(())
}
