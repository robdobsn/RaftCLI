// RaftCLI: Serial monitor module
// Rob Dobson 2024

use crossterm::{
    execute,
    event::{poll, read, Event, KeyCode},
    terminal::{self, Clear, ClearType, enable_raw_mode, disable_raw_mode},
    cursor::{MoveTo, MoveToNextLine},
};
use std::{str, io};
use std::io::{stdout, Write};
use std::sync::Arc;
use bytes::{BufMut, BytesMut};
use tokio_util::codec::{Decoder, Encoder};
use tokio::sync::{oneshot, Mutex};
use tokio_util::codec::Framed;
use tokio_serial::SerialPortBuilderExt;
use tokio_serial::SerialStream;
use futures::stream::{SplitSink, StreamExt};
use futures::stream::SplitStream;
use futures::SinkExt;
struct LineCodec;
use std::process::{Command, Stdio};

struct LogFileInfo {
    file: std::fs::File,
    last_write: std::time::Instant
}
type SharedLogFile = Arc<Mutex<Option<LogFileInfo>>>;

// Logging to file
fn open_log_file(log_to_file: bool, log_folder: String) -> Result<SharedLogFile, std::io::Error> {
    if log_to_file && log_folder.len() > 0 && log_folder != "none" {
        // Create a log file
        // name YYYYMMDD-HHMMSS.log (eg. 20210923-123456.log)
        let name = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
        let log_file_name = format!("{}/{}.log", log_folder, name);
        std::fs::create_dir_all(log_folder)?;
        // Open the log file
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_file_name);
        return Ok(Arc::new(Mutex::new(Some(
            LogFileInfo {
                file: file?,
                last_write: std::time::Instant::now()
            }
        ))));
    }
    Ok(Arc::new(Mutex::new(None)))
}

// Write to log file and maybe close/reopen
async fn write_and_maybe_rotate_log(log_file: &SharedLogFile, item: &str) -> std::io::Result<()> {
    let mut log_file = log_file.lock().await;
    if let Some(log_file) = log_file.as_mut() {

        // Write to log file
        write!(log_file.file, "{item}")?;

        // Check elapsed time
        if log_file.last_write.elapsed() > std::time::Duration::from_secs(1) { // 1 seconds threshold
            // Close and reopen the log file
            log_file.file.sync_all()?;
        }

        // Update last write time
        log_file.last_write = std::time::Instant::now();
    }
    Ok(())
}

// Convert key codes to terminal sequences
fn key_code_to_terminal_sequence(key_code: KeyCode) -> String {
    match key_code {
        KeyCode::Enter => "\r".to_string(), // Carriage return
        KeyCode::Backspace => "\x08".to_string(), // Backspace
        KeyCode::Left => "\x1b[D".to_string(), // ANSI escape sequence for left arrow
        KeyCode::Right => "\x1b[C".to_string(), // ANSI escape sequence for right arrow
        KeyCode::Up => "\x1b[A".to_string(), // ANSI escape sequence for up arrow
        KeyCode::Down => "\x1b[B".to_string(), // ANSI escape sequence for down arrow
        KeyCode::Char(c) => c.to_string(), // Direct character input
        KeyCode::Tab => "\t".to_string(), // Horizontal tab
        KeyCode::Esc => "\x1b".to_string(), // Escape
        // Add more key mappings here as needed
        _ => "".to_string(), // Unsupported keys return an empty string
    }
}

// Open serial port
fn open_serial_port(port: &str, baud: u32) -> tokio_serial::Result<tokio_serial::SerialStream> {

    // Serial port builder
    let mut serial_port_builder = tokio_serial::new(port, baud);
    serial_port_builder = serial_port_builder.stop_bits(tokio_serial::StopBits::One);
    let serial_port = serial_port_builder.open_native_async();

    // Handle errors in opening the serial port
    match serial_port {
        Ok(serial_port) => {

            // This is to get around mutability issues
            #[cfg(unix)]
            let mut serial_port = serial_port;

            // Set the port to non-exclusive mode on unix-based OSs
            #[cfg(unix)]
            serial_port.set_exclusive(false).expect("Failed to set port non-exclusive");

            // Return the serial port
            Ok(serial_port)
        }
        Err(err) => {
            match err.kind() {
                tokio_serial::ErrorKind::NoDevice => {
                    println!("Error opening serial port {} - is the device connected?", port);
                },
                _ => {
                    println!("Error opening serial port {} {:?}", port, err);
                }
            }
            Err(err)
        }
    }
}

// Implement the tokio codec decoder for line-based communication
impl Decoder for LineCodec {
    type Item = String;
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        let newline = src.as_ref().iter().position(|b| *b == b'\n');
        if let Some(n) = newline {
            let line = src.split_to(n + 1);
            return match str::from_utf8(line.as_ref()) {
                Ok(s) => Ok(Some(s.to_string())),
                Err(_) => Err(io::Error::new(io::ErrorKind::Other, "Invalid String")),
            };
        }
        Ok(None)
    }
}

// Implement the tokio codec encoder for line-based communication
impl Encoder<String> for LineCodec {
    type Error = io::Error;

    fn encode(&mut self, item: String, dst: &mut BytesMut) -> Result<(), Self::Error> {
        // println!("In writer {:?}", &item);
        dst.reserve(item.len() + 1);
        dst.put(item.as_bytes());
        dst.put_u8(b'\r');
        Ok(())
    }
}

// Serial monitor read from serial port and write to terminal
fn read_from_serial_port_and_write_terminal(
    user_input_buffer: &Arc<Mutex<String>>,
    mut serial_rx: SplitStream<Framed<SerialStream, LineCodec>>,
    log_file: SharedLogFile,
) {

    // Clone the user input buffer for use in the serial_rx task
    let serial_rx_buffer_clone = user_input_buffer.clone();

    // Clone the log file for use in the serial_rx task
    let log_file_clone = log_file.clone();

    // Create a task to read from the serial port and send to the terminal
    tokio::spawn(async move {
        loop {
            match serial_rx.next().await {
                Some(Ok(item)) => {
                    // Log to file if required
                    if let Err(e) = write_and_maybe_rotate_log(&log_file_clone, &item).await {
                        eprintln!("Failed to write to log file: {}", e);
                        // Ignore errors for now
                    }

                    // Get the terminal output
                    let mut stdout = stdout();

                    // Check if the user input buffer is not empty and display it
                    {
                        // Lock the user input buffer
                        let buf = serial_rx_buffer_clone.lock().await;

                        // If it isn't empty then delete the bottom row before continuing
                        if !buf.is_empty() {
                            execute!(stdout, MoveToNextLine(1), Clear(ClearType::CurrentLine)).unwrap();
                        }

                        // Print the received serial data
                        print!("{item}");

                        // Check if the user input buffer is not empty
                        if !buf.is_empty() {

                            // Move to the start of the line
                            execute!(stdout, MoveTo(0, terminal::size().unwrap().1 - 1)).unwrap();

                            // Output the user input buffer to the blank line
                            print!("{}", buf);
                        }
                    } // Release the lock on the user input buffer

                    if stdout.flush().is_err() {
                        eprintln!("Failed to flush stdout");
                        // Handle the error as needed
                    }
                },
                Some(Err(_e)) => {
                    // eprintln!("Failed to read from RX stream: {}", _e);
                    // Ignore errors
                    continue;
                },
                None => {
                    // eprintln!("RX stream ended");
                    // Ignore errors
                    continue;
                },
            }
        }
    });    
}

// Read from the terminal and write to the serial port
fn read_from_terminal_and_write_to_serial_port(user_input_buffer: &Arc<Mutex<String>>, 
    mut serial_tx: SplitSink<Framed<SerialStream, LineCodec>, String>,
    oneshot_exit_send: oneshot::Sender<()>) {

    // Clone the user input buffer for use in the serial_tx task
    let serial_tx_buffer_clone = user_input_buffer.clone();

    // Create a task to read from the terminal and send to the serial port
    tokio::spawn(async move {

        // Stdout
        let mut stdout = stdout();

        // Main serial monitor loop
        loop {

            if poll(tokio::time::Duration::from_millis(100)).expect("Error polling for event") {
                let evt = read().expect("Error reading event");
                match evt {
                    Event::Key(key) => {
                        // Check for key press (release events are also available)
                        if key.kind != crossterm::event::KeyEventKind::Press {
                            continue;
                        }

                        // Handle key press
                        {
                            // Lock the user input buffer
                            let mut buf = serial_tx_buffer_clone.lock().await;

                            // Handle key press
                            match key.code {
                                // Break out of the serial monitor on Esc key or Ctrl+X
                                KeyCode::Esc => {
                                    let _ = oneshot_exit_send.send(());
                                    break;
                                },
                                KeyCode::Char('x') if key.modifiers == crossterm::event::KeyModifiers::CONTROL => {
                                    let _ = oneshot_exit_send.send(());
                                    break;
                                },
                                // Check for Enter key and send the user input buffer
                                KeyCode::Enter => {
                                    // Clear the user input display line before sending.
                                    execute!(stdout, MoveTo(0, terminal::size().unwrap().1 - 1), Clear(ClearType::CurrentLine)).unwrap();

                                    // Add a carriage return to the user input buffer
                                    buf.push_str("\r");

                                    // Send the user input buffer to the serial port
                                    let write_result = serial_tx
                                        .send(buf.clone())
                                        .await;
                                    match write_result {
                                        Ok(_) => (),
                                        Err(err) => println!("{:?}", err),
                                    }

                                    // Clear the user input buffer
                                    buf.clear();
                                },
                                // Handle backspace
                                KeyCode::Backspace => {
                                    // Pop the last character from the buffer
                                    buf.pop();
                                    // Clear the user input display line
                                    execute!(stdout, MoveTo(0, terminal::size().unwrap().1 - 1), Clear(ClearType::CurrentLine)).unwrap();
                                    // Display the buffer as the user types.
                                    print!("{}", buf);
                                },
                                // Handle other characters
                                _ => {
                                    buf.push_str(key_code_to_terminal_sequence(key.code).as_str());
                                    // Display the buffer as the user types.
                                    print!("{}", key_code_to_terminal_sequence(key.code));
                                }
                            }
                        } // Release the lock on the user input buffer

                        // Ensure the user's typing appears at the bottom line.
                        stdout.flush().unwrap();
                    },
                    _ => {} // Handle other events here
                }
            }
        }
    });
}

// Start the serial monitor
pub async fn start_native(port: String, baud: u32, log: bool, log_folder: String) -> tokio_serial::Result<()> {

    // TODO - move

    // Open serial port
    let serial_port = open_serial_port(&port, baud)?;

    // Create a stream from the serial port
    let stream = LineCodec.framed(serial_port);
    let (serial_tx, serial_rx) = stream.split();
    
    // Debug
    // println!("Starting serial monitor on port: {} at baud: {}", port, baud);

    // Open log file if required
    let log_file = if log {
        let file = open_log_file(log, log_folder.clone())?;
        file
    } else {
        Arc::new(Mutex::new(None))
    };

    // User input buffer
    let user_input_buffer = Arc::new(Mutex::new(String::new()));

    // Enter crossterm raw mode (characters are not automatically echoed to the terminal)
    enable_raw_mode()?;

    // Setup signaling mechanism
    let (oneshot_exit_send, oneshot_exit_get) = oneshot::channel();
    
    // Write welcome message to the terminal
    let version = env!("CARGO_PKG_VERSION");  
    println!("Raft Serial Monitor {} - press Esc (or Ctrl+X) to exit", version);
    
    // Start the process to read from serial port and write to terminal
    read_from_serial_port_and_write_terminal(&user_input_buffer, serial_rx, log_file);

    // Start the process to read from terminal and write to serial port
    read_from_terminal_and_write_to_serial_port(&user_input_buffer, serial_tx, oneshot_exit_send);

    // Wait here for the oneshot signal to exit
    let _ = oneshot_exit_get.await;

    // Exit crossterm raw mode
    disable_raw_mode()?;

    Ok(())
}

pub fn start_non_native(port: String, baud: u32, log: bool, log_folder: String) -> Result<(), Box<dyn std::error::Error>> {

    // Setup args
    let mut args = vec!["monitor".to_string(), "-p".to_string(), port, "-b".to_string(), baud.to_string()];
    if log {
        args.push("--log".to_string());
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
                Ok(status) => println!("Process exited with status: {}", status),
                Err(e) => println!("Error waiting for serial monitor: {:?}", e),
            }
        },
        Err(e) => println!("Error starting serial monitor: {:?}", e),
    }

    Ok(())
}
