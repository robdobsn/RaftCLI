// RaftCLI: Minimal HTTP client helpers (no external HTTP crate)
// Used by filesystem (app_fs) and reusable by OTA.
// Rob Dobson 2024

use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

// Default read/write timeout
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

// Connecting: a connection attempt now and then goes unanswered (seen about once in 1600 from WSL), and without a
// timeout the system's own (about two minutes on Linux) applies. Nothing has been sent when connecting fails, so it is
// safe to try again, whatever the request
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const CONNECT_ATTEMPTS: u32 = 3;

// Connect (trying again, up to attempts times, if the connection isn't made), with the read and write timeout set
fn connect(ip_addr: &str, port: u16, timeout: Duration, attempts: u32) -> io::Result<TcpStream> {
    use std::net::ToSocketAddrs;
    let addr = format!("{}:{}", ip_addr, port)
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("can't find {}", ip_addr)))?;
    let mut last_err = None;
    for _ in 0..attempts.max(1) {
        match TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT.min(timeout)) {
            Ok(stream) => {
                stream.set_read_timeout(Some(timeout))?;
                stream.set_write_timeout(Some(timeout))?;
                return Ok(stream);
            }
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.unwrap_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "connect failed")))
}

// Result of an HTTP request
pub struct HttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

impl HttpResponse {
    pub fn body_string(&self) -> String {
        String::from_utf8_lossy(&self.body).to_string()
    }
    pub fn is_success(&self) -> bool {
        self.status >= 200 && self.status < 300
    }
}

// Parse the status line and split headers/body from a raw HTTP response buffer
fn parse_http_response(raw: &[u8]) -> Result<HttpResponse, io::Error> {
    // Find header/body separator
    let sep = b"\r\n\r\n";
    let header_end = raw
        .windows(sep.len())
        .position(|w| w == sep)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Malformed HTTP response (no header terminator)"))?;
    let header_bytes = &raw[..header_end];
    let body = raw[header_end + sep.len()..].to_vec();

    // Parse status code from the first line e.g. "HTTP/1.1 200 OK"
    let header_str = String::from_utf8_lossy(header_bytes);
    let first_line = header_str.lines().next().unwrap_or("");
    let status = first_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Malformed HTTP status line"))?;

    Ok(HttpResponse { status, body })
}

// Perform an HTTP GET and return the response (reads until connection close)
pub fn http_get(ip_addr: &str, port: u16, path: &str) -> Result<HttpResponse, Box<dyn std::error::Error>> {
    let mut stream = connect(ip_addr, port, DEFAULT_TIMEOUT, CONNECT_ATTEMPTS)?;

    let request = format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nAccept: */*\r\n\r\n",
        path, ip_addr
    );
    stream.write_all(request.as_bytes())?;
    stream.flush()?;

    let mut raw = Vec::new();
    stream.read_to_end(&mut raw)?;
    Ok(parse_http_response(&raw)?)
}

// Progress callback: (bytes_sent_so_far, total_bytes, rate_bytes_per_sec)
pub type ProgressFn<'a> = dyn FnMut(u64, u64, f64) + 'a;

// Stream a file as a multipart/form-data POST with progress reporting.
// Returns the response on completion.
pub fn http_post_multipart_file(
    ip_addr: &str,
    port: u16,
    path: &str,
    field_name: &str,
    upload_filename: &str,
    file_path: &str,
    mut progress: Option<&mut ProgressFn>,
) -> Result<HttpResponse, Box<dyn std::error::Error>> {
    use std::fs::File;
    use std::io::BufReader;

    let metadata = std::fs::metadata(file_path)?;
    let file_size = metadata.len();
    let file = File::open(file_path)?;
    let mut reader = BufReader::new(file);

    let mut stream = connect(ip_addr, port, DEFAULT_TIMEOUT, CONNECT_ATTEMPTS)?;

    // Multipart envelope
    let boundary = "----RaftCLIBoundary7c3a1f9e";
    let start_boundary = format!("--{}\r\n", boundary);
    let content_disposition = format!(
        "Content-Disposition: form-data; name=\"{}\"; filename=\"{}\"\r\n",
        field_name, upload_filename
    );
    let content_type_part = "Content-Type: application/octet-stream\r\n\r\n";
    let end_boundary = format!("\r\n--{}--\r\n", boundary);

    let headers_length = start_boundary.len() + content_disposition.len() + content_type_part.len();
    let content_length = headers_length + file_size as usize + end_boundary.len();

    let request = format!(
        "POST {} HTTP/1.1\r\n\
         Host: {}\r\n\
         Content-Type: multipart/form-data; boundary={}\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\r\n",
        path, ip_addr, boundary, content_length
    );

    stream.write_all(request.as_bytes())?;
    stream.write_all(start_boundary.as_bytes())?;
    stream.write_all(content_disposition.as_bytes())?;
    stream.write_all(content_type_part.as_bytes())?;
    stream.flush()?;

    // Stream the file body in chunks, reporting progress
    let mut buf = vec![0u8; 4096];
    let mut sent: u64 = 0;
    let start = Instant::now();
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        stream.write_all(&buf[..n])?;
        sent += n as u64;
        if let Some(cb) = progress.as_deref_mut() {
            let elapsed = start.elapsed().as_secs_f64();
            let rate = if elapsed > 0.0 { sent as f64 / elapsed } else { 0.0 };
            cb(sent, file_size, rate);
        }
    }
    stream.write_all(end_boundary.as_bytes())?;
    stream.flush()?;

    let mut raw = Vec::new();
    stream.read_to_end(&mut raw)?;
    Ok(parse_http_response(&raw)?)
}

// HTTP GET with a connect and read timeout (e.g. to poll a device that is restarting)
pub fn http_get_timeout(ip_addr: &str, port: u16, path: &str, timeout: Duration) -> Result<HttpResponse, Box<dyn std::error::Error>> {
    // Once (the caller polls)
    let mut stream = connect(ip_addr, port, timeout, 1)?;

    let request = format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nAccept: */*\r\n\r\n",
        path, ip_addr
    );
    stream.write_all(request.as_bytes())?;
    stream.flush()?;

    let mut raw = Vec::new();
    stream.read_to_end(&mut raw)?;
    Ok(parse_http_response(&raw)?)
}
