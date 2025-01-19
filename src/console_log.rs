use std::fs::{File, OpenOptions};
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::Instant;

pub struct LogFileInfo {
    pub file: File,
    pub last_write: Instant,
}

pub type SharedLogFile = Arc<Mutex<Option<LogFileInfo>>>;

/// Opens a log file for writing. Creates the folder if it doesn't exist.
pub fn open_log_file(log_to_file: bool, log_folder: &str) -> Result<SharedLogFile, std::io::Error> {
    if log_to_file && !log_folder.is_empty() && log_folder != "none" {
        // Create log folder if needed
        std::fs::create_dir_all(log_folder)?;

        // Generate log file name with timestamp
        let timestamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
        let log_file_path = format!("{}/{}.log", log_folder, timestamp);

        // Open log file
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_file_path)?;

        Ok(Arc::new(Mutex::new(Some(LogFileInfo {
            file,
            last_write: Instant::now(),
        }))))
    } else {
        Ok(Arc::new(Mutex::new(None)))
    }
}

/// Writes a message to the log file.
pub fn write_to_log(log_file: &SharedLogFile, message: &str) {
    if let Ok(mut log_file_lock) = log_file.lock() {
        if let Some(log_file_info) = log_file_lock.as_mut() {
            if let Err(e) = writeln!(log_file_info.file, "{}", message) {
                eprintln!("Failed to write to log file: {}", e);
            }
            log_file_info.last_write = Instant::now();
        }
    }
}
