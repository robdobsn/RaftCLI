// RaftCLI: Serial monitor module
// Rob Dobson 2024

use crossterm::{
    event::{poll, read, Event, KeyCode},
    terminal::{enable_raw_mode, disable_raw_mode},
};
use futures::stream::StreamExt;
use std::{io, str};
use tokio::time::Duration;
use tokio_util::codec::{Decoder, Encoder};
use tokio::sync::oneshot;
use futures::SinkExt;

use bytes::{BufMut, BytesMut};
use tokio_serial::SerialPortBuilderExt;
struct LineCodec;

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
        dst.reserve(item.len());
        dst.put(item.as_bytes());
        Ok(())
    }
}

// Start the serial monitor
pub async fn start(port: String, baud: u32) -> tokio_serial::Result<()> {

    // Debug
    println!("Starting serial monitor on port: {} at baud: {}", port, baud);

    // Enter crossterm raw mode (characters are not automatically echoed to the terminal)
    enable_raw_mode()?;

    // Setup signaling mechanism
    let (oneshot_exit_tx, oneshot_exit_rx) = oneshot::channel();

    // Open serial port
    let serial_port = tokio_serial::new(port, baud).open_native_async()?;

    #[cfg(unix)]
    port.set_exclusive(false).expect("Failed to set port exclusive");

    let stream = LineCodec.framed(serial_port);
    let (mut tx, mut rx) = stream.split();

    tokio::spawn(async move {
        loop {
            let item = rx
                .next()
                .await
                .expect("Error awaiting future in RX stream.")
                .expect("Reading stream resulted in an error");
            print!("{item}");
        }
    });

    tokio::spawn(async move {
        loop {

            if poll(Duration::from_millis(100)).expect("Error polling for event") {
                let evt = read().expect("Error reading event");
                match evt {
                    Event::Key(key) => {
                        // println!("Key pressed: {:?}", key);
                        // Exit loop if 'q' is pressed
                        if key.code == KeyCode::Esc {
                            let _ = oneshot_exit_tx.send(());
                            break;
                        }

                        if key.kind == crossterm::event::KeyEventKind::Press {
                            let msg = key_code_to_terminal_sequence(key.code);
                            let write_result = tx
                                .send(msg)
                                .await;
                            match write_result {
                                Ok(_) => (),
                                Err(err) => println!("{:?}", err),
                            }
                        }
                    },
                    _ => {} // Handle other events here
                }
            }
        }
    });

    // Wait here for the oneshot signal to exit
    let _ = oneshot_exit_rx.await;

    // Exit crossterm raw mode
    disable_raw_mode()?;

    Ok(())
}

#[cfg(target_os = "macos")]
pub fn get_default_port() -> String {
    "/dev/tty.usbserial".to_string()
}

#[cfg(target_os = "windows")]
pub fn get_default_port() -> String {
    "COM3".to_string()
}

#[cfg(target_os = "linux")]
pub fn get_default_port() -> String {
    "/dev/ttyUSB0".to_string()
}
