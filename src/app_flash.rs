use crate::app_ports::select_most_likely_port;
use crate::app_ports::PortsCmd;
use crate::raft_cli_utils::build_flash_command_args;
use crate::raft_cli_utils::get_flash_tool_cmd;
use crate::raft_cli_utils::execute_and_capture_output;
// use crate::raft_cli_utils::get_device_type;
use crate::raft_cli_utils::get_build_folder_name;
use crate::raft_cli_utils::utils_get_sys_type;
use reqwest::blocking::{Client, multipart};
use reqwest::header::{HeaderMap, HeaderValue};
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

// Updated ProgressReader to own the file
struct ProgressReader<R> {
    inner: R, // Store the file directly
    chunk_size: usize,
    total_read: u64,
    progress: Arc<Mutex<ProgressTracker>>,
}

impl<R: Read> ProgressReader<R> {
    fn new(inner: R, chunk_size: usize, progress: Arc<Mutex<ProgressTracker>>) -> Self {
        Self { inner, chunk_size, total_read: 0, progress }
    }
}

impl<R: Read> Read for ProgressReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let read_size = std::cmp::min(self.chunk_size, buf.len());
        let n = self.inner.read(&mut buf[..read_size])?;
        self.total_read += n as u64;
        println!("Read {} bytes total {}", n, self.total_read);
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

// Perform the OTA flash with progress tracking
fn perform_ota_flash_sync_with_progress(
    fw_image_path: &str,
    fw_image_name: &str,
    ip_addr: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    // Check if the firmware file exists
    if !Path::new(fw_image_path).exists() {
        return Err(Box::new(std::io::Error::new(
            std::io::ErrorKind::NotFound,
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
    let reader = ProgressReader::new(file, 2880, progress_tracker.clone());

    // Create a multipart form with the custom progress-tracking reader
    let file_part = multipart::Part::reader(reader)
        .file_name(fw_image_name.to_string())
        .mime_str("application/octet-stream")?;

    let form = multipart::Form::new().part("file", file_part);

    // Create custom headers
    let mut headers = HeaderMap::new();
    headers.insert(reqwest::header::CONTENT_TYPE, HeaderValue::from_static("multipart/form-data"));
    // headers.insert(reqwest::header::EXPECT, HeaderValue::from_static("100-continue"));
    headers.insert(reqwest::header::CONTENT_LENGTH, HeaderValue::from(file_size));

    // Create a reqwest client with increased timeout and custom headers
    let client = Client::builder()
        .timeout(Duration::from_secs(300))
        .tcp_nodelay(true)  // Disable Nagle's algorithm to send small packets immediately
        .default_headers(headers)
        .build()?;

    println!("Attempting to send OTA request to: http://{}/api/espFwUpdate", ip_addr);

    // Perform the HTTP POST request with reqwest (blocking)
    let response = client
        .post(format!("http://{}/api/espFwUpdate", ip_addr))
        .multipart(form)
        .send();

    // Print detailed information about the response
    match response {
        Ok(resp) => {
            let status = resp.status(); // Extract status before calling `text()`
            let text = resp.text()?; // Move `resp` here by calling `text()`
            println!("Response Status: {:?}", status);
            println!("Response Text: {:?}", text);

            if status.is_success() {
                println!("OTA flash successful");
                Ok(())
            } else {
                println!("OTA flash failed: {:?}", text);
                Err(Box::new(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("OTA flash failed: {:?}", text),
                )))
            }
        }
        Err(e) => {
            println!("Request failed with error: {:?}", e);
            Err(Box::new(e))
        }
    }
}

pub fn flash_raft_app(
    build_sys_type: &Option<String>,
    app_folder: String,
    ip_addr: Option<String>,
    port: Option<String>,
    native_serial_port: bool,
    vid: Option<String>,
    flash_baud: u32,
    flash_tool_opt: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {

    let sys_type = utils_get_sys_type(build_sys_type, app_folder.clone());
    if sys_type.is_err() {
        return Err(Box::new(std::io::Error::new(std::io::ErrorKind::Other, "Error determining SysType")));
    }
    let sys_type = sys_type.unwrap();

    // Get build folder
    let build_folder = get_build_folder_name(sys_type.clone(), app_folder.clone());

    // If IP address is specified, perform OTA flashing
    if let Some(ip_addr) = &ip_addr {
        let fw_image_name = format!("{}.bin", sys_type);
        let fw_image_path = format!("{}/build/{}/{}", app_folder, sys_type, fw_image_name);
        println!("Flashing {} FW image is {}", sys_type, fw_image_path);

        // Call the synchronous version of perform_ota_flash with progress tracking
        match perform_ota_flash_sync_with_progress(&fw_image_path, &fw_image_name, ip_addr) {
            Ok(_) => println!("OTA flash successful"),
            Err(e) => println!("OTA flash failed: {:?}", e),
        }
        
        // let ota_result = std::process::Command::new("curl")
        //     .arg("-F")
        //     .arg(format!("file=@{}", fw_image_path))  // Ensure this uses the correct app folder path
        //     .arg(format!("http://{}/api/espFwUpdate", ip_addr))
        //     .output();

        // if let Ok(output) = ota_result {
        //     if output.status.success() {
        //         println!("OTA flash successful");
        //         return Ok(());
        //     } else {
        //         println!("OTA flash failed: {}", String::from_utf8_lossy(&output.stderr));
        //         return Err("Failed to execute curl command".to_string().into());
        //     }
        // } else {
        //     return Err("Failed to execute curl command".to_string().into());
        // }


    } else {

        // Get flash tool
        let flash_cmd: String = get_flash_tool_cmd(flash_tool_opt, native_serial_port);

        // Extract port and baud rate arguments
        let port = if let Some(port) = port {
            port
        } else {
            // Use select_most_likely_port if no specific port is provided
            let port_cmd = PortsCmd::new_with_vid(vid);
            match select_most_likely_port(&port_cmd, native_serial_port) {
                Some(p) => p.port_name,
                None => {
                    println!("Error: No suitable port found");
                    std::process::exit(1);
                }
            }
        };

        // Extract the arguments for the flash command
        let flash_cmd_args = build_flash_command_args(build_folder.clone(), &port, flash_baud);

        // Check for errors in the flash command and arguments
        if flash_cmd_args.is_err() {
            return Err(Box::new(std::io::Error::new(
                std::io::ErrorKind::Other,
                "Error extracting flash command arguments",
            )));
        }
        let flash_cmd_args = flash_cmd_args.unwrap();

        // Debug
        println!("Flash command: {}", flash_cmd.clone());
        println!("Flash command args: {:?}", flash_cmd_args);
        println!("Flash command app folder: {}", app_folder.clone());
        println!("Flash command build folder: {}", build_folder);

        // Execute the flash command and check for errors
        let (output, success_flag) = execute_and_capture_output(flash_cmd.clone(), &flash_cmd_args, build_folder.clone())?;
        if !success_flag {
            let err_msg = format!("Flash executed with errors: {}", output);
            return Err(Box::new(std::io::Error::new(std::io::ErrorKind::Other, err_msg)));
        }
    }

    Ok(())
}