use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyModifiers},
    execute,
    style::{Color, Print, ResetColor, SetForegroundColor},
    terminal::{self, Clear, ClearType},
};
use std::{io::{self, Read, Write}, sync::atomic::{AtomicBool, Ordering}};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;

pub fn start_debug_console<A: ToSocketAddrs>(
    server_address: A,
) -> Result<(), Box<dyn std::error::Error>> {
    // Connect to the server and clone the stream for separate read/write
    let stream = TcpStream::connect(server_address)?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let stream_reader = Arc::new(Mutex::new(stream.try_clone()?)); // Separate reader
    let stream_writer = Arc::new(Mutex::new(stream)); // Separate writer

    // Set up terminal for raw mode
    terminal::enable_raw_mode()?;
    execute!(io::stdout(), Clear(ClearType::All), cursor::MoveTo(0, 0))?;

    let running= Arc::new(AtomicBool::new(true));
    let running_clone = running.clone(); // Clone for use in threads

    // Channels for handling incoming and outgoing messages
    let (input_tx, input_rx): (mpsc::Sender<String>, mpsc::Receiver<String>) = mpsc::channel();
    let (output_tx, output_rx): (mpsc::Sender<String>, mpsc::Receiver<String>) = mpsc::channel();

    // Thread for receiving messages from the server
    {
        let stream_reader = Arc::clone(&stream_reader);
        let running_clone = Arc::clone(&running_clone);
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
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(50)); // Avoid busy-waiting
                    }
                    Err(_) => {
                        break; // Handle disconnection or critical errors
                    }
                }
            }
        });
    }

    // Thread for sending messages to the server
    {
        let stream_writer = Arc::clone(&stream_writer);
        thread::spawn(move || {
            while let Ok(message) = input_rx.recv() {
                println!("Sending message: {}", message);
                let mut stream = stream_writer.lock().unwrap();
                match stream.write(format!("{}\n", message).as_bytes()) {
                    Ok(_) => println!("Message sent to server: {}", message),
                    Err(e) => {
                        println!("Failed to send message: {}", e);
                        break;
                    }
                }
                stream
                    .flush()
                    .unwrap_or_else(|e| println!("Flush failed: {}", e));
            }
        });
    }

    let mut command_buffer = String::new();
    let mut cursor_row = 0; // Track the current row for scrolling

    // Main event loop for the terminal UI
    while running.load(Ordering::SeqCst) {
        // Display incoming messages and scroll
        if let Ok(message) = output_rx.try_recv() {
            // Move to the next row for the new message
            execute!(
                io::stdout(),
                cursor::MoveTo(0, cursor_row),
                Clear(ClearType::CurrentLine),
                Print(message)
            )?;
            io::stdout().flush()?;
            cursor_row += 1;

            // Scroll when the output reaches the bottom of the terminal
            let (_, rows) = terminal::size()?;
            if cursor_row >= rows - 1 {
                cursor_row = rows - 2; // Keep cursor within bounds
                execute!(io::stdout(), cursor::MoveTo(0, 0), terminal::ScrollUp(1))?;
            }

            // Keep the command buffer visible
            execute!(
                io::stdout(),
                cursor::MoveTo(0, rows - 1),
                Clear(ClearType::CurrentLine),
                SetForegroundColor(Color::Yellow),
                Print(format!("> {}", command_buffer)),
                ResetColor
            )?;
        }

        // Handle user input
        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(key_event) = event::read()? {
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
                        println!("Received ENTER Sending command: {}", command_buffer);
                        input_tx
                            .send(command_buffer.clone()) // Send the command
                            .expect("Failed to send command");
                        command_buffer.clear(); // Clear the buffer after sending
                    }
                    KeyCode::Backspace => {
                        if !command_buffer.is_empty() {
                            command_buffer.pop(); // Remove the last character
                        }
                    }
                    KeyCode::Char(c) => {
                        command_buffer.push(c); // Append the new character
                    }
                    _ => {}
                }

                // Refresh the command buffer display
                let (_, rows) = terminal::size()?; // Get terminal dimensions
                execute!(
                    io::stdout(),
                    cursor::MoveTo(0, rows - 1), // Move to the last line
                    Clear(ClearType::CurrentLine), // Clear the line
                    SetForegroundColor(Color::Yellow), // Set text color
                    Print(format!("> {}", command_buffer)), // Display the buffer
                    ResetColor                   // Reset text color
                )?;
            }
        }
    }

    terminal::disable_raw_mode()?; // Restore the terminal to normal mode
    println!("Exiting debug console...");
    Ok(())
}
