// RaftCLI: Serial monitor module
// Rob Dobson 2024

use crossterm::{
    event::{self, Event, KeyEventKind}, terminal,
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

use crate::{app_ports::{select_most_likely_port, PortsCmd}, console_log::{open_log_file, write_to_log}, terminal_io::TerminalIO};

struct CommandAndTime {
    user_input: String,
    _time: std::time::Instant,
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
    let log_file = if log {
        let file = open_log_file(log, &log_folder)?;
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
    let port = if let Some(port) = serial_port_name {
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

    // Command history in the app folder
    let history_file_path = format!("{}/{}", app_folder, history_file_name);
    
    // Terminal output
    let terminal_io = Arc::new(Mutex::new(TerminalIO::new(&history_file_path)));
    terminal_io.lock().unwrap().init().unwrap();

    // Clone the Arc for the terminal output
    let terminal_io_clone = Arc::clone(&terminal_io);

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
                    write_to_log(&log_file, &received);
                }
                Ok(_) => {}
                Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => {}
                Err(_e) => {
                    terminal_io_clone.lock().unwrap().show_error("Serial port read error");
                    if no_reconnect {
                        break;
                    }
                    terminal_io_clone.lock().unwrap().show_error("Serial port attempting to reconnect...");
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
    terminal_io.lock().unwrap().print("", false);

    // Main loop to handle terminal events and print received serial data
    while running.load(Ordering::SeqCst) {
        // Handle serial data
        if let Ok(received) = serial_read_rx.try_recv() {
            terminal_io.lock().unwrap().print(&received, true);
        }
    
        // Handle keyboard input
        if crossterm::event::poll(Duration::from_millis(50))? {
            if let Event::Key(key_event) = event::read()? {
                if key_event.kind == KeyEventKind::Press {
                    let mut terminal_io = terminal_io.lock().unwrap();
                    let continue_running = terminal_io.handle_key_event(
                        key_event,
                        |command| {
                            let key_detect_time = std::time::Instant::now();
                            let command_to_send = CommandAndTime {
                                user_input: command.clone(),
                                _time: key_detect_time,
                            };
                            serial_write_tx
                                .send(command_to_send)
                                .expect("Failed to send command to write thread");
                        },
                    );
                    if !continue_running {
                        running.store(false, Ordering::SeqCst);
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
