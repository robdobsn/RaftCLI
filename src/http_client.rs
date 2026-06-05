// RaftCLI: Minimal HTTP client helpers (no external HTTP crate)
// Used by filesystem (app_fs) and reusable by OTA.
// Rob Dobson 2024

use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

// Default connect/read timeout
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

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
    let addr = format!("{}:{}", ip_addr, port);
    let mut stream = TcpStream::connect(&addr)?;
    stream.set_read_timeout(Some(DEFAULT_TIMEOUT))?;
    stream.set_write_timeout(Some(DEFAULT_TIMEOUT))?;

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

    let addr = format!("{}:{}", ip_addr, port);
    let mut stream = TcpStream::connect(&addr)?;
    stream.set_read_timeout(Some(DEFAULT_TIMEOUT))?;
    stream.set_write_timeout(Some(DEFAULT_TIMEOUT))?;

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
