// RaftCLI: RICREST over a WebSocket
// Each binary message is an HDLC frame (RaftWebServer's websockets with pcol RICSerial, at ws://<host>/ws). As on
// serial, the socket is read only in recv, on the calling thread; pings from the device are answered as they are
// read.
// Rob Dobson 2026

use std::collections::VecDeque;
use std::io::ErrorKind;
use std::net::{TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

use tungstenite::protocol::WebSocket;
use tungstenite::Message;

use super::{encode_wire, Link, WireDecoder, WireFormat};
use crate::device_conn::CmdResult;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);
// The shortest read timeout (a zero timeout means "don't block" to the socket)
const MIN_READ_TIMEOUT: Duration = Duration::from_millis(1);

pub struct WebSocketLink {
    url: String,
    host: String,
    port: u16,
    socket: Option<WebSocket<TcpStream>>,
    decoder: WireDecoder,
    frames: VecDeque<Vec<u8>>,
}

fn connect(url: &str, host: &str, port: u16) -> CmdResult<WebSocket<TcpStream>> {
    let addr = (host, port)
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| format!("can't find {}", host))?;
    let stream = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT).map_err(|e| format!("can't connect to {}: {}", url, e))?;
    stream.set_write_timeout(Some(WRITE_TIMEOUT))?;
    stream.set_read_timeout(Some(CONNECT_TIMEOUT))?;
    let (socket, _) = tungstenite::client(url, stream).map_err(|e| {
        format!("{} didn't accept a WebSocket ({}) - is the device's web server running with a RICSerial websocket?", url, e)
    })?;
    Ok(socket)
}

impl WebSocketLink {
    pub fn open(host: &str, port: u16, path: &str) -> CmdResult<WebSocketLink> {
        let url = format!("ws://{}:{}{}", host, port, path);
        let socket = connect(&url, host, port)?;
        Ok(WebSocketLink {
            url,
            host: host.to_string(),
            port,
            socket: Some(socket),
            decoder: WireDecoder::new(WireFormat::Hdlc),
            frames: VecDeque::new(),
        })
    }

    fn closed(&self) -> String {
        format!("WebSocket {} closed (the device restarted or the network dropped)", self.url)
    }

    // Read one message (waiting up to `timeout`), decoding frames into self.frames
    fn read_message(&mut self, timeout: Duration) -> CmdResult<()> {
        let Some(socket) = self.socket.as_mut() else {
            return Err(self.closed().into());
        };
        socket.get_ref().set_read_timeout(Some(timeout.max(MIN_READ_TIMEOUT)))?;
        match socket.read() {
            Ok(Message::Binary(data)) => {
                let (mut frames, mut text) = (Vec::new(), Vec::new());
                self.decoder.feed(&data, &mut frames, &mut text);
                self.frames.extend(frames);
                Ok(())
            }
            Ok(Message::Close(_)) => {
                self.socket = None;
                Err(self.closed().into())
            }
            // Text, ping and pong messages (pings are answered by tungstenite)
            Ok(_) => Ok(()),
            Err(tungstenite::Error::Io(e)) if e.kind() == ErrorKind::WouldBlock || e.kind() == ErrorKind::TimedOut => Ok(()),
            Err(_) => {
                self.socket = None;
                Err(self.closed().into())
            }
        }
    }
}

impl Link for WebSocketLink {
    fn send(&mut self, frame: &[u8]) -> CmdResult<()> {
        let url = &self.url;
        let socket = self.socket.as_mut().ok_or_else(|| format!("WebSocket {} isn't open", url))?;
        socket
            .send(Message::Binary(encode_wire(WireFormat::Hdlc, frame).into()))
            .map_err(|e| format!("sending on WebSocket {}: {}", url, e).into())
    }

    fn recv(&mut self, timeout: Duration) -> CmdResult<Option<Vec<u8>>> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(frame) = self.frames.pop_front() {
                return Ok(Some(frame));
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            self.read_message(remaining)?;
            if remaining.is_zero() && self.frames.is_empty() {
                return Ok(None);
            }
        }
    }

    fn bad_frames(&self) -> u32 {
        self.decoder.bad_frames()
    }

    // After a restart the device's web server comes back once it is on the network again: connect again
    fn reopen(&mut self, timeout: Duration) -> bool {
        if let Some(mut socket) = self.socket.take() {
            let _ = socket.close(None);
        }
        self.frames.clear();
        let start = Instant::now();
        std::thread::sleep(Duration::from_millis(1500));
        while start.elapsed() < timeout {
            if let Ok(socket) = connect(&self.url, &self.host, self.port) {
                self.socket = Some(socket);
                return true;
            }
            std::thread::sleep(Duration::from_millis(1000));
        }
        false
    }
}

impl Drop for WebSocketLink {
    fn drop(&mut self) {
        if let Some(mut socket) = self.socket.take() {
            let _ = socket.close(None);
            let _ = socket.flush();
        }
    }
}
