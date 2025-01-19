use crossterm::{
    event::{self, Event, KeyEventKind},
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

use crate::terminal_io::TerminalIO;

pub fn start_debug_console<A: ToSocketAddrs>(
    app_folder: String,
    server_address: A,
    history_file_name: String,
) -> Result<(), Box<dyn std::error::Error>> {
    // Connect to the server and clone the stream for separate read/write
    let stream = TcpStream::connect(server_address)?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let stream_reader = Arc::new(Mutex::new(stream.try_clone()?)); // Separate reader
    let stream_writer = Arc::new(Mutex::new(stream)); // Separate writer

    // Command history in the app folder
    let history_file_path = format!("{}/{}", app_folder, history_file_name);
    let terminal_out = Arc::new(Mutex::new(TerminalIO::new(&history_file_path)));
    terminal_out.lock().unwrap().init()?; // Initialize terminal

    let running = Arc::new(AtomicBool::new(true));
    let running_clone = running.clone(); // Clone for use in threads

    // Channels for handling incoming and outgoing messages
    let (input_tx, input_rx): (mpsc::Sender<String>, mpsc::Receiver<String>) = mpsc::channel();
    let (output_tx, output_rx): (mpsc::Sender<String>, mpsc::Receiver<String>) = mpsc::channel();

    // Thread for receiving messages from the server
    {
        let stream_reader = Arc::clone(&stream_reader);
        let running_clone = Arc::clone(&running_clone);
        let terminal_out = Arc::clone(&terminal_out);

        thread::spawn(move || {
            let mut buffer = [0; 512];
            while running_clone.load(Ordering::SeqCst) {
                let mut stream = stream_reader.lock().unwrap();
                match stream.read(&mut buffer) {
                    Ok(bytes_read) if bytes_read > 0 => {
                        let message = String::from_utf8_lossy(&buffer[..bytes_read]).to_string();
                        output_tx.send(message).expect("Failed to send message");
                    }
                    Ok(_) => {} // No data received
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock 
                               || e.kind() == std::io::ErrorKind::TimedOut => {
                        // Expected timeout, continue looping
                    }
                    Err(_) => {
                        // Handle disconnection or critical errors
                        break;
                    }
                }
            }
            terminal_out
                .lock()
                .unwrap()
                .show_error("Disconnected from server.");
        });
    }

    // Thread for sending messages to the server
    {
        let stream_writer = Arc::clone(&stream_writer);

        thread::spawn(move || {
            while let Ok(message) = input_rx.recv() {
                let mut stream = stream_writer.lock().unwrap();
                if stream.write(format!("{}\n", message).as_bytes()).is_err() {
                    break;
                }
                stream.flush().unwrap_or_else(|e| println!("Flush failed: {}", e));
            }
        });
    }

    // Main event loop for the terminal UI
    while running.load(Ordering::SeqCst) {
        // Display incoming messages
        if let Ok(message) = output_rx.try_recv() {
            terminal_out.lock().unwrap().print(&message, true);
        }

        // Handle keyboard input
        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(key_event) = event::read()? {
                if key_event.kind == KeyEventKind::Press {
                    let mut terminal_out = terminal_out.lock().unwrap();
                    let continue_running = terminal_out.handle_key_event(
                        key_event,
                        |command| {
                            input_tx
                                .send(command.clone())
                                .expect("Failed to send command");
                        },
                    );
                    if !continue_running {
                        running.store(false, Ordering::SeqCst);
                    }
                }
            }
        }
    }

    terminal::disable_raw_mode()?; // Restore the terminal to normal mode
    println!("Exiting debug console...");
    Ok(())
}
