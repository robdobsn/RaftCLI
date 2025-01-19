use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyModifiers},
    execute,
    style::{Color, Print, ResetColor, SetForegroundColor},
    terminal::{self, Clear, ClearType},
};
use std::io::{self, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;

pub fn start_debug_console<A: ToSocketAddrs>(
    server_address: A,
) -> Result<(), Box<dyn std::error::Error>> {
    // Connect to the server
    let stream = TcpStream::connect(server_address)?;
    let stream = Arc::new(Mutex::new(stream)); // Wrap TcpStream in Arc<Mutex<>> for shared access

    // Set up terminal for raw mode
    terminal::enable_raw_mode()?;
    execute!(
        io::stdout(),
        Clear(ClearType::All),
        cursor::MoveTo(0, 0)
    )?;

    let running = Arc::new(Mutex::new(true));
    let running_clone = running.clone(); // Clone for use in threads

    // Channels for handling incoming and outgoing messages
    let (input_tx, input_rx): (mpsc::Sender<String>, mpsc::Receiver<String>) = mpsc::channel();
    let (output_tx, output_rx) = mpsc::channel();

    // Thread for receiving messages from the server
    {
        let stream_clone = Arc::clone(&stream);
        let running_clone = Arc::clone(&running_clone);
        thread::spawn(move || {
            let mut buffer = [0; 512];
            while *running_clone.lock().unwrap() {
                let mut stream = stream_clone.lock().unwrap();
                match stream.read(&mut buffer) {
                    Ok(bytes_read) if bytes_read > 0 => {
                        let message = String::from_utf8_lossy(&buffer[..bytes_read]);
                        output_tx.send(message.to_string()).expect("Failed to send message");
                    }
                    Ok(_) => {}
                    Err(e) => {
                        eprintln!("Error reading from server: {}", e);
                        break;
                    }
                }
            }
        });
    }

    // Thread for sending messages to the server
    {
        let stream_clone = Arc::clone(&stream);
        thread::spawn(move || {
            while let Ok(message) = input_rx.recv() {
                let mut stream = stream_clone.lock().unwrap();
                if stream.write(message.as_bytes()).is_err() {
                    eprintln!("Error sending message to server");
                    break;
                }
            }
        });
    }

    let mut command_buffer = String::new();
    let mut cursor_position = 0;

    // Main event loop for the terminal UI
    while *running.lock().unwrap() {
        // Display incoming messages
        if let Ok(message) = output_rx.try_recv() {
            execute!(
                io::stdout(),
                Clear(ClearType::All),
                cursor::MoveTo(0, 0)
            )?;
            print!("{}", message);
            io::stdout().flush()?;
        }

        // Handle user input
        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(key_event) = event::read()? {
                match key_event.code {
                    KeyCode::Esc | KeyCode::Char('c') if key_event.modifiers == KeyModifiers::CONTROL => {
                        *running.lock().unwrap() = false;
                    }
                    KeyCode::Enter => {
                        input_tx
                            .send(command_buffer.clone())
                            .expect("Failed to send command");
                        command_buffer.clear();
                        cursor_position = 0;
                    }
                    KeyCode::Backspace => {
                        if cursor_position > 0 {
                            command_buffer.pop();
                            cursor_position -= 1;
                        }
                    }
                    KeyCode::Char(c) => {
                        command_buffer.push(c);
                        cursor_position += 1;
                    }
                    _ => {}
                }

                // Refresh the command buffer display
                execute!(
                    io::stdout(),
                    cursor::MoveTo(0, terminal::size()?.1 - 1),
                    Clear(ClearType::CurrentLine),
                    SetForegroundColor(Color::Yellow),
                    Print(format!("> {}", command_buffer)),
                    ResetColor
                )?;
            }
        }
    }

    terminal::disable_raw_mode()?;
    println!("Exiting debug console...");
    Ok(())
}
