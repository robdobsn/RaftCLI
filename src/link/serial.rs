// RaftCLI: RICREST over a serial port
// Frames are OverAscii(HDLC) on the device's console port (SerialConsole, protocol RICSerial), sharing it with the
// device's log text. The port is read only while waiting for a frame (in recv), on the calling thread: on Windows a
// reader thread on a cloned port handle holds up writes for seconds at a time, as synchronous reads and writes on
// one handle wait for each other. Log text read on the way goes to the text sink. The port is opened without
// changing DTR or RTS, so boards with an auto-reset circuit aren't reset.
// Rob Dobson 2026

use std::collections::VecDeque;
use std::io::{ErrorKind, Read, Write};
use std::time::{Duration, Instant};

use serialport::SerialPort;

use super::{encode_wire, Link, WireDecoder, WireFormat};
use crate::device_conn::CmdResult;

// How long each read waits for data (recv reads until its own timeout)
const READ_TIMEOUT: Duration = Duration::from_millis(20);
// How long a write may wait. USB Serial/JTAG holds the host off while the device's receive buffer is full (e.g.
// while it writes a block to flash), and a write that gave up part-way would leave half a frame on the wire
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);

// Where the device's log text goes
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextSink {
    Drop,
    Stderr,
}

pub struct SerialLink {
    port_name: String,
    baud: u32,
    text: TextSink,
    // None while reopening, or after the port has gone (the device restarted or was unplugged)
    port: Option<Box<dyn SerialPort>>,
    decoder: WireDecoder,
    frames: VecDeque<Vec<u8>>,
}

fn open_port(port_name: &str, baud: u32) -> CmdResult<Box<dyn SerialPort>> {
    // DTR is left as it is (serialport's default): changing it on open can reset a board
    serialport::new(port_name, baud).timeout(READ_TIMEOUT).open().map_err(|e| {
        let busy = matches!(e.kind(), serialport::ErrorKind::Io(ErrorKind::PermissionDenied))
            || e.to_string().to_lowercase().contains("busy")
            || e.to_string().to_lowercase().contains("access is denied");
        if busy {
            format!("serial port {} is in use - close raft monitor (or whatever else has it open)", port_name).into()
        } else {
            format!("can't open serial port {}: {}", port_name, e).into()
        }
    })
}

impl SerialLink {
    pub fn open(port_name: &str, baud: u32, text: TextSink) -> CmdResult<SerialLink> {
        Ok(SerialLink {
            port_name: port_name.to_string(),
            baud,
            text,
            port: Some(open_port(port_name, baud)?),
            decoder: WireDecoder::new(WireFormat::OverAsciiHdlc),
            frames: VecDeque::new(),
        })
    }

    fn closed(&self) -> String {
        format!("serial port {} closed (the device restarted or was unplugged)", self.port_name)
    }

    // Read what is there (waiting up to READ_TIMEOUT if `wait`), decoding frames into self.frames
    fn read_some(&mut self, wait: bool) -> CmdResult<()> {
        let Some(port) = self.port.as_mut() else {
            return Err(self.closed().into());
        };
        if !wait && port.bytes_to_read().unwrap_or(0) == 0 {
            return Ok(());
        }
        let mut buf = [0u8; 4096];
        match port.read(&mut buf) {
            Ok(n) => {
                let (mut frames, mut text) = (Vec::new(), Vec::new());
                self.decoder.feed(&buf[..n], &mut frames, &mut text);
                if self.text == TextSink::Stderr && !text.is_empty() {
                    let _ = std::io::stderr().write_all(&text);
                }
                self.frames.extend(frames);
                Ok(())
            }
            Err(e) if e.kind() == ErrorKind::TimedOut || e.kind() == ErrorKind::Interrupted => Ok(()),
            Err(_) => {
                self.port = None;
                Err(self.closed().into())
            }
        }
    }
}

impl Link for SerialLink {
    fn send(&mut self, frame: &[u8]) -> CmdResult<()> {
        let port_name = &self.port_name;
        let port = self.port.as_mut().ok_or_else(|| format!("serial port {} isn't open", port_name))?;
        // The port has one timeout, for reads and writes
        let _ = port.set_timeout(WRITE_TIMEOUT);
        let result = port.write_all(&encode_wire(WireFormat::OverAsciiHdlc, frame)).and_then(|_| port.flush());
        let _ = port.set_timeout(READ_TIMEOUT);
        result.map_err(|e| format!("writing to serial port {}: {}", port_name, e).into())
    }

    fn recv(&mut self, timeout: Duration) -> CmdResult<Option<Vec<u8>>> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(frame) = self.frames.pop_front() {
                return Ok(Some(frame));
            }
            let waiting = Instant::now() < deadline;
            self.read_some(waiting)?;
            if !waiting && self.frames.is_empty() {
                return Ok(None);
            }
        }
    }

    fn wire_len(&self, frame: &[u8]) -> usize {
        encode_wire(WireFormat::OverAsciiHdlc, frame).len()
    }

    fn bad_frames(&self) -> u32 {
        self.decoder.bad_frames()
    }

    // A native USB port goes away while the device restarts and comes back (with the same name on Windows and
    // macOS; Linux may give it another); a USB-UART bridge's port stays. Either way, open it again
    fn reopen(&mut self, timeout: Duration) -> bool {
        // Close it first: a port can only be open once (on Windows)
        self.port = None;
        self.frames.clear();
        let start = Instant::now();
        // Give the device time to go down first
        std::thread::sleep(Duration::from_millis(1500));
        while start.elapsed() < timeout {
            if let Ok(port) = open_port(&self.port_name, self.baud) {
                self.port = Some(port);
                return true;
            }
            std::thread::sleep(Duration::from_millis(500));
        }
        false
    }
}
