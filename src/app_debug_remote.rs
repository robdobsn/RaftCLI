use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    terminal,
};
use std::{
    io::{Read, Write},
    net::{TcpStream, ToSocketAddrs},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    thread,
    time::Duration,
};

use crate::{
    console_log::{open_log_file, write_to_log, SharedLogFile},
    terminal_io::TerminalIO,
};

pub fn connect_to_server(
    server_address: &impl ToSocketAddrs,
) -> Result<TcpStream, Box<dyn std::error::Error>> {
    let stream = TcpStream::connect(server_address)?;
    stream.set_nonblocking(true)?; // Set non-blocking mode
    Ok(stream)
}

pub fn setup_threads(
    running: Arc<AtomicBool>,
    disconnected: Arc<AtomicBool>,
    stream: TcpStream,
    input_rx: Arc<Mutex<mpsc::Receiver<String>>>,
    output_tx: mpsc::Sender<String>,
    terminal_out: Arc<Mutex<TerminalIO>>,
    log_file: SharedLogFile,
) {
    let stream_reader = Arc::new(Mutex::new(stream.try_clone().unwrap())); // Separate reader
    let stream_writer = Arc::new(Mutex::new(stream)); // Separate writer

    // Thread for receiving messages from the server
    {
        let stream_reader = Arc::clone(&stream_reader);
        let running_clone = Arc::clone(&running);
        let disconnected_clone = Arc::clone(&disconnected);
        let terminal_out = Arc::clone(&terminal_out);

        thread::spawn(move || {
            let mut buffer = [0; 512];
            while running_clone.load(Ordering::SeqCst) {
                let mut stream = stream_reader.lock().unwrap();
                match stream.read(&mut buffer) {
                    Ok(bytes_read) if bytes_read > 0 => {
                        let received = String::from_utf8_lossy(&buffer[..bytes_read]).to_string();
                        output_tx
                            .send(received.clone())
                            .expect("Failed to send data");
                        write_to_log(&log_file, &received);
                    }
                    Ok(_) => {} // No data received
                    Err(ref e)
                        if e.kind() == std::io::ErrorKind::WouldBlock
                            || e.kind() == std::io::ErrorKind::TimedOut =>
                    {
                        // Expected timeout, continue looping
                    }
                    Err(_) => {
                        // Signal disconnection and exit thread
                        disconnected_clone.store(true, Ordering::SeqCst);
                        break;
                    }
                }
            }
            terminal_out
                .lock()
                .unwrap()
                .show_error("Disconnected from device.");
        });
    }

    // Thread for sending messages to the server
    {
        let stream_writer = Arc::clone(&stream_writer);
        let running_clone = Arc::clone(&running);
        let disconnected_clone = Arc::clone(&disconnected);

        thread::spawn(move || {
            while running_clone.load(Ordering::SeqCst) {
                if disconnected_clone.load(Ordering::SeqCst) {
                    break;
                }
                if let Ok(message) = input_rx.lock().unwrap().recv() {
                    let mut stream = stream_writer.lock().unwrap();
                    if stream.write(format!("{}\n", message).as_bytes()).is_err() {
                        disconnected_clone.store(true, Ordering::SeqCst);
                        break;
                    }
                    stream
                        .flush()
                        .unwrap_or_else(|e| println!("Flush failed: {}", e));
                }
            }
        });
    }
}

pub fn start_debug_console<A: ToSocketAddrs>(
    app_folder: String,
    server_address: A,
    log: bool,
    log_folder: String,
    history_file_name: String,
) -> Result<(), Box<dyn std::error::Error>> {
    // Open log file if required
    let log_file = open_log_file(log, &log_folder)?;

    // Command history in the app folder
    let history_file_path = format!("{}/{}", app_folder, history_file_name);
    let terminal_out = Arc::new(Mutex::new(TerminalIO::new(&history_file_path)));
    terminal_out.lock().unwrap().init()?; // Initialize terminal

    let running = Arc::new(AtomicBool::new(true));
    let disconnected = Arc::new(AtomicBool::new(false));

    // Channels for handling incoming and outgoing messages
    let (input_tx, input_rx): (mpsc::Sender<String>, mpsc::Receiver<String>) = mpsc::channel();
    let (output_tx, output_rx): (mpsc::Sender<String>, mpsc::Receiver<String>) = mpsc::channel();

    let input_rx = Arc::new(Mutex::new(input_rx)); // Wrap input_rx in Arc<Mutex<>> for reuse in threads.

    while running.load(Ordering::SeqCst) {
        terminal_out
            .lock()
            .unwrap()
            .show_info("Connecting to device...");
        match connect_to_server(&server_address) {
            Ok(stream) => {
                disconnected.store(false, Ordering::SeqCst); // Reset disconnection signal
                setup_threads(
                    Arc::clone(&running),
                    Arc::clone(&disconnected),
                    stream,
                    Arc::clone(&input_rx),
                    output_tx.clone(),
                    Arc::clone(&terminal_out),
                    Arc::clone(&log_file),
                );

                terminal_out.lock().unwrap().clear_info();

                // Main event loop for the terminal UI
                while running.load(Ordering::SeqCst) && !disconnected.load(Ordering::SeqCst) {
                    // Display incoming messages
                    if let Ok(message) = output_rx.try_recv() {
                        terminal_out.lock().unwrap().print(&message, true);
                    }

                    // Handle keyboard input
                    if event::poll(Duration::from_millis(50))? {
                        if let Event::Key(key_event) = event::read()? {
                            if key_event.kind == KeyEventKind::Press {
                                let mut terminal_out = terminal_out.lock().unwrap();
                                let continue_running =
                                    terminal_out.handle_key_event(key_event, |command| {
                                        input_tx
                                            .send(command.clone())
                                            .expect("Failed to send command");
                                    });
                                if !continue_running {
                                    running.store(false, Ordering::SeqCst);
                                }
                            }
                        }
                    }
                }
            }
            Err(e) => {
                terminal_out
                    .lock()
                    .unwrap()
                    .show_error(&format!("Error: {}. Retrying in 5 seconds...", e));

                let retry_interval = Duration::from_secs(5);
                let poll_interval = Duration::from_millis(50);
                let mut elapsed = Duration::ZERO;

                while elapsed < retry_interval {
                    // Check for keyboard input
                    if event::poll(poll_interval)? {
                        if let Event::Key(key_event) = event::read()? {
                            if key_event.kind == KeyEventKind::Press {
                                match key_event.code {
                                    KeyCode::Char('c') | KeyCode::Char('x')
                                        if key_event.modifiers == KeyModifiers::CONTROL =>
                                    {
                                        running.store(false, Ordering::SeqCst);
                                        break;
                                    }
                                    KeyCode::Esc => {
                                        running.store(false, Ordering::SeqCst);
                                        break;
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                    // Increment elapsed time
                    elapsed += poll_interval;
                }
            }
        }
    }

    terminal::disable_raw_mode()?; // Restore the terminal to normal mode
    println!("Exiting debug console...");
    Ok(())
}
