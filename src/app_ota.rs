use crate::raft_cli_utils::get_build_folder_name;
use crate::raft_cli_utils::utils_get_sys_type;
use std::fs::File;
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::sync::{Arc, Mutex};

// ProgressReader implementation to track file upload progress
struct ProgressReader<R> {
    inner: R,
    chunk_size: usize,
    total_read: u64,
    progress: Arc<Mutex<ProgressTracker>>,
}

impl<R: Read> ProgressReader<R> {
    fn new(inner: R, chunk_size: usize, progress: Arc<Mutex<ProgressTracker>>) -> Self {
        Self {
            inner,
            chunk_size,
            total_read: 0,
            progress,
        }
    }
}

impl<R: Read> Read for ProgressReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let read_size = std::cmp::min(self.chunk_size, buf.len());
        let n = self.inner.read(&mut buf[..read_size])?;
        self.total_read += n as u64;
        println!("Read {} bytes, total: {}", n, self.total_read);
        let mut progress = self.progress.lock().unwrap();
        progress.update(n);
        Ok(n)
    }
}

// Progress tracking structure
struct ProgressTracker {
    total_size: u64,
    bytes_read: u64,
}

impl ProgressTracker {
    fn new(total_size: u64) -> Self {
        Self {
            total_size,
            bytes_read: 0,
        }
    }

    fn update(&mut self, bytes: usize) {
        self.bytes_read += bytes as u64;
        let percentage = (self.bytes_read as f64 / self.total_size as f64) * 100.0;
        println!(
            "Progress: {:.2}% | {}/{} bytes",
            percentage, self.bytes_read, self.total_size
        );
    }
}

fn perform_ota_flash_basic_http(
    fw_image_path: &str,
    fw_image_name: &str,
    ip_addr: &str,
    port: u16,
) -> Result<(), Box<dyn std::error::Error>> {
    // Check if the firmware file exists
    if !Path::new(fw_image_path).exists() {
        return Err(Box::new(io::Error::new(
            io::ErrorKind::NotFound,
            "Firmware image not found",
        )));
    }

    // Get the file size for progress tracking
    let metadata = std::fs::metadata(fw_image_path)?;
    let file_size = metadata.len();

    // Open the file and create a progress tracker
    let file = File::open(fw_image_path)?;
    let progress_tracker = Arc::new(Mutex::new(ProgressTracker::new(file_size)));

    // Create a ProgressReader that owns the file
    let mut reader = ProgressReader::new(file, 2880, progress_tracker.clone());

    // Connect to the server
    let addr = format!("{}:{}", ip_addr, port);
    let mut stream = TcpStream::connect(&addr)?;
    println!("Connected to {}", addr);

    // Construct the multipart body
    let boundary = "----CustomBoundary123456";
    let start_boundary = format!("--{}\r\n", boundary);
    let content_disposition = format!(
        "Content-Disposition: form-data; name=\"file\"; filename=\"{}\"\r\n",
        fw_image_name
    );
    let content_type = "Content-Type: application/octet-stream\r\n\r\n";
    let end_boundary = format!("\r\n--{}--\r\n", boundary);

    // Read the file content into a buffer
    let mut file_content = Vec::new();
    reader.read_to_end(&mut file_content)?;

    // Create the complete HTTP body
    let mut body_content = Vec::new();
    body_content.extend_from_slice(start_boundary.as_bytes());
    body_content.extend_from_slice(content_disposition.as_bytes());
    body_content.extend_from_slice(content_type.as_bytes());
    body_content.extend_from_slice(&file_content);
    body_content.extend_from_slice(end_boundary.as_bytes());

    // Calculate Content-Length
    let content_length = body_content.len();

    // Create HTTP POST request manually
    let request = format!(
        "POST /api/espFwUpdate HTTP/1.1\r\n\
         Host: {}\r\n\
         Content-Type: multipart/form-data; boundary={}\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\r\n",
        ip_addr, boundary, content_length
    );

    // Write request headers to stream
    stream.write_all(request.as_bytes())?;
    // Write body content to stream
    stream.write_all(&body_content)?;

    // Read the response
    let mut response = String::new();
    stream.read_to_string(&mut response)?;
    println!("Response: {}", response);

    // Check response for success
    if response.contains("200 OK") {
        println!("OTA flash successful");
    } else {
        println!("OTA flash failed with response: {}", response);
    }

    Ok(())
}

pub fn ota_raft_app(
    build_sys_type: &Option<String>,
    app_folder: String,
    ip_addr: String,
    ip_port: Option<u16>,
    use_curl: bool,
) -> Result<(), Box<dyn std::error::Error>> {

    // Get the system type
    let sys_type = utils_get_sys_type(build_sys_type, app_folder.clone());
    if sys_type.is_err() {
        return Err(Box::new(std::io::Error::new(std::io::ErrorKind::Other, "Error determining SysType")));
    }

    // Unwrap the sys_type, ip_addr, and ip_port
    let sys_type = sys_type.unwrap();
    let ip_port = ip_port.unwrap_or(80);
    let fw_image_name = format!("{}.bin", sys_type);
    let fw_image_path = format!("{}/build/{}/{}", app_folder, sys_type, fw_image_name);

    // Check if not using curl
    if !use_curl {
        println!("Flashing {} FW image is {}", sys_type, fw_image_path);

        // Call the synchronous version of perform_ota_flash with progress tracking
        match perform_ota_flash_basic_http(&fw_image_path, &fw_image_name, &ip_addr, ip_port) {
            Ok(_) => println!("OTA flash successful"),
            Err(e) => println!("OTA flash failed: {:?}", e),
        }

    } else {

        // Use curl to perform OTA flashing
        let ota_result = std::process::Command::new("curl")
            .arg("-F")
            .arg(format!("file=@{}", fw_image_path))  // Ensure this uses the correct app folder path
            .arg(format!("http://{}/api/espFwUpdate", ip_addr))
            .output();

        if let Ok(output) = ota_result {
            if output.status.success() {
                println!("OTA flash successful");
                return Ok(());
            } else {
                println!("OTA flash failed: {}", String::from_utf8_lossy(&output.stderr));
                return Err("Failed to execute curl command".to_string().into());
            }
        } else {
            return Err("Failed to execute curl command".to_string().into());
        }
    }

    Ok(())
}