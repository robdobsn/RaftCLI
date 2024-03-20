use std::path::Path;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::fs;
use std::error::Error;
use regex::Regex;
use std::fmt::{self, Display, Formatter};
use std::io::{self, BufRead, BufReader};
use std::sync::{Arc, Mutex};
use remove_dir_all::remove_dir_contents;
use crossbeam::thread;

#[cfg(not(target_os = "windows"))]
use std::env;

pub fn utils_get_sys_type(build_sys_type: &Option<String>, app_folder: &str) -> Result<String, Box<dyn std::error::Error>> {

    // Determine the Systype to build - this is either the SysType passed in or
    // the first SysType found in the systypes folder (excluding Common)
    let mut sys_type: String = String::new();
    if let Some(build_sys_type) = build_sys_type {
        sys_type = build_sys_type.to_string();
    } else {
        let sys_types = fs::read_dir(
            format!("{}/{}", app_folder, get_systypes_folder_name())
        );
        if sys_types.is_err() {
            println!("Error reading the systypes folder: {}", sys_types.err().unwrap());
            return Err(Box::new(std::io::Error::new(std::io::ErrorKind::Other, "Error reading the systypes folder")));
        }
        for sys_type_dir_entry in sys_types.unwrap() {
            let sys_type_dir = sys_type_dir_entry;
            if sys_type_dir.is_err() {
                println!("Error reading the systypes folder: {}", sys_type_dir.err().unwrap());
                return Err(Box::new(std::io::Error::new(std::io::ErrorKind::Other, "Error reading the systypes folder")));
            }
            let sys_type_name = sys_type_dir.unwrap().file_name().into_string().unwrap();
            if sys_type_name != "Common" {
                sys_type = sys_type_name;
                break;
            }
        }
    }

    Ok(sys_type)
}

// Check the app folder is valid
pub fn check_app_folder_valid(app_folder: &str) -> bool {
    // The app folder is valid if it exists and contains a CMakeLists.txt file
    // and a folder called systypes 
    let cmake_file = format!("{}/CMakeLists.txt", app_folder);
    if !Path::new(&app_folder).exists() {
        println!("Error: app folder does not exist: {}", app_folder);
        false
    } else if !Path::new(&cmake_file).exists() {
        println!("Error: app folder does not contain a CMakeLists.txt file: {}", app_folder);
        false
    } else if !Path::new(&format!("{}/{}", app_folder, get_systypes_folder_name())).exists() {
        println!("Error: app folder does not contain a systypes folder: {}", app_folder);
        false
    } else {
        true
    }
}

pub fn check_for_raft_artifacts_deletion(app_folder: &str, sys_type: &str) -> bool {
    // Check if the "build_raft_artifacts" folder exists inside the app folder
    // and if so extract the contents of the "cursystype.txt" file to determine
    // the SysType of the last build - then check if this is the same as the
    // sys_type to build and if not, delete the "build_raft_artifacts" folder
    let build_raft_artifacts_folder = format!("{}/build_raft_artifacts", app_folder);
    if Path::new(&build_raft_artifacts_folder).exists() {
        let cursystype_file = format!("{}/cursystype.txt", build_raft_artifacts_folder);
        if Path::new(&cursystype_file).exists() {
            let cursystype = fs::read_to_string(&cursystype_file);
            if cursystype.is_err() {
                println!("Error reading the cursystype.txt file: {}", cursystype.err().unwrap());
                return true;
            }
            if cursystype.unwrap().trim() != sys_type {
                println!("Delete the build_raft_artifacts folder as the SysType to build has changed");
                return true;
            }
        }
        else
        {
            println!("Delete the build_raft_artifacts folder as the cursystype.txt file is missing");
            return true;
        }
    }
    false
}

pub fn convert_path_for_docker(path: PathBuf) -> Result<String, std::io::Error> {
    let path_str = path.into_os_string().into_string().unwrap();

    // Remove the '\\?\' prefix if present (Windows extended-length path)
    let trimmed_path = if path_str.starts_with("\\\\?\\") {
        &path_str[4..]
    } else {
        &path_str
    };

    // Replace backslashes with forward slashes
    let docker_path = trimmed_path.replace("\\", "/");

    // Debug
    println!("Converted path: {} to: {}", path_str, docker_path);

    Ok(docker_path)
}

// Define an enum for different error types
#[derive(Debug)]
pub enum CommandError {
    CommandNotFound(String),
    ExecutionFailed(String),
    Other(io::Error),
}

impl Display for CommandError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        // Implementation details here, for example:
        write!(f, "{:?}", self) // Simple placeholder implementation
    }
}

impl Error for CommandError {}

pub fn execute_and_capture_output(command: &str, args: &Vec<String>, cur_dir: &str) -> Result<(String, bool), CommandError> {

    let process = Command::new(command)
        .current_dir(cur_dir)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();

    // Match on the result
    let mut process = match process {
        Ok(process) => process,
        Err(e) => {
            if e.kind() == io::ErrorKind::NotFound {
                return Err(CommandError::CommandNotFound(format!("{}: No such file or directory", command)));
            } else {
                return Err(CommandError::Other(e));
            }
        },
    };

    // Capture the output
    let stdout = process.stdout.take().unwrap();
    let stderr = process.stderr.take().unwrap();

    let stdout_reader = BufReader::new(stdout);
    let stderr_reader = BufReader::new(stderr);

    let captured_output = Arc::new(Mutex::new(String::new()));

    // Using crossbeam to handle threads
    let thread_result = thread::scope(|s| {
        let captured = Arc::clone(&captured_output);
        s.spawn(move |_| {
            for line in stdout_reader.lines() {
                match line {
                    Ok(line) => {
                        println!("{}", line); // Print to console
                        let mut captured = captured.lock().unwrap();
                        captured.push_str(&line);
                        captured.push('\n');
                    }
                    Err(_) => break,
                }
            }
        });

        let captured = Arc::clone(&captured_output);
        s.spawn(move |_| {
            for line in stderr_reader.lines() {
                match line {
                    Ok(line) => {
                        eprintln!("{}", line); // Print to console
                        let mut captured = captured.lock().unwrap();
                        captured.push_str(&line);
                        captured.push('\n');
                    }
                    Err(_) => break,
                }
            }
        });
    });

    // Handle thread problems
    if thread_result.is_err() {
        return Err(CommandError::ExecutionFailed("Failed to execute threads".into()));
    }
    
    // Wait for the process to finish
    let output = captured_output.lock().unwrap().clone();
    let success_flag = process.wait().unwrap().success();
    Ok((output, success_flag))
}

fn get_systypes_folder_name() -> &'static str {
    // systypes folder name
    "systypes"
}

// Check if running a linux binary under WSL
pub fn is_wsl() -> bool {

    // If this is a windows binary then return false
    #[cfg(target_os = "windows")]
    {
        return false;
    }

    #[cfg(not(target_os = "windows"))]
    {
        // If the WSL_DISTRO_NAME environment variable is set then return true
        if env::var("WSL_DISTRO_NAME").is_ok() {
            return true;
        }

        // If the /proc/version file contains "Microsoft" or "WSL" then return true
        // For instance this may be the string returned ...
        // Linux version 5.15.146.1-microsoft-standard-WSL2 (root@65c757a075e2) (gcc (GCC) 11.2.0, GNU ld (GNU Binutils) 2.37) #1 SMP Thu Jan 11 04:09:03 UTC 2024
        let proc_version = fs::read_to_string("/proc/version");
        if proc_version.is_ok() {
            return proc_version.as_ref().unwrap().contains("Microsoft") || proc_version.unwrap().contains("WSL");
        }
        return false;
    }
}

pub fn get_flash_tool_cmd(flash_tool_opt: Option<String>, native_serial_port: bool) -> String {

    // If the tool is specified then use it, otherwise determine the tool from the platform
    match flash_tool_opt {
        Some(tool) => tool,
        None => {
            if !native_serial_port && is_wsl() {
                "esptool.py.exe".to_string()
            } else {
                "esptool.py".to_string()
            }
        }
    }
}

pub fn extract_flash_cmd_args(output: String, port: &str, flash_baud: u32) -> 
                    Result<Vec<String>, Box<dyn std::error::Error>> {

    // The result contains the command to flash the app which will look something like:
    // /opt/esp/python_env/idf5.1_py3.8_env/bin/python ../opt/esp/idf/components/esptool_py/esptool/esptool.py -p (PORT) -b 460800 --before default_reset --after hard_reset --chip esp32  write_flash --flash_mode dio --flash_size 4MB --flash_freq 40m 0x1000 build/SysTypeMain/bootloader/bootloader.bin 0x8000 build/SysTypeMain/partition_table/partition-table.bin 0x1e000 build/SysTypeMain/ota_data_initial.bin 0x20000 build/SysTypeMain/SysTypeMain.bin 0x380000 build/SysTypeMain/fs.bin
    // OR
    // python -m esptool --chip esp32 -b 460800 --before default_reset --after hard_reset write_flash --flash_mode dio --flash_size 4MB --flash_freq 40m 0x1000 build/ShadesScader/bootloader/bootloader.bin 0x8000 build/ShadesScader/partition_table/partition-table.bin 0x1e000 build/ShadesScader/ota_data_initial.bin 0x20000 build/ShadesScader/ShadesScader.bin 0x380000 build/ShadesScader/fs.bin
    // Extract the command to flash the app using the esptool.py as the keyword to locate the start of the command

    // Create a regex pattern to match "esptool.py " or "esptool "
    let re = Regex::new(r"esptool\.py |esptool ").unwrap();

    // Find the match and get the start of the command
    let flash_command_start = re.find(&output)
        .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "Flash command not found in output"))?
        .start();

    // Extract the command starting from the located placeholder
    let mut flash_command = output[flash_command_start..].to_string();
    
    // Truncate the command at the first newline character, if present
    if let Some(end) = flash_command.find('\n') {
        flash_command.truncate(end);
    }

    // Remove "esptool" or "esptool.py" from the start of the command
    let esptool_regex = Regex::new("esptool(\\.py)?").map_err(|e| e.to_string())?;
    flash_command = esptool_regex.replace(&flash_command, "").to_string();

    // Remove the "-p (PORT)" if it exists
    let port_regex = Regex::new("-p \\(PORT\\)").map_err(|e| e.to_string())?;
    flash_command = port_regex.replace(&flash_command, "").to_string();

    // Remove the "-b {{flash_baud}}" if it exists
    let baud_regex = Regex::new("-b \\d+").map_err(|e| e.to_string())?;
    flash_command = baud_regex.replace(&flash_command, "").to_string();
    
    // The required arguments for flashing the app will look something like this
    // -p {{port}} -b {{flash_baud}} --before default_reset --after hard_reset --chip esp32  write_flash --flash_mode dio --flash_size 4MB --flash_freq 40m 0x1000 build/SysTypeMain/bootloader/bootloader.bin 0x8000 build/SysTypeMain/partition_table/partition-table.bin 0x1e000 build/SysTypeMain/ota_data_initial.bin 0x20000 build/SysTypeMain/SysTypeMain.bin 0x380000 build/SysTypeMain/fs.bin

    // Create the string to prepend - it should be -p {{port}} -b {{flash_baud}}
    let flash_command_prepend = format!("-p {} -b {}", port, flash_baud);

    // Prepend the required arguments to the command
    flash_command = format!("{} {}", flash_command_prepend, flash_command);
    
    // Split the modified command into parts for use as arguments
    let flash_command_parts: Vec<String> = flash_command.split_whitespace()
                                                        .map(String::from)
                                                        .collect();

    println!("Flash command parts: {:?}", flash_command_parts);
    
    Ok(flash_command_parts)
}

// TODO - make these default to value read from config file in project folder

#[cfg(target_os = "macos")]
pub fn get_default_port(_native_serial_port: bool) -> String {
    "/dev/tty.usbserial".to_string()
}

#[cfg(target_os = "windows")]
pub fn get_default_port(_native_serial_port: bool) -> String {
    "COM3".to_string()
}

#[cfg(target_os = "linux")]
pub fn get_default_port(_native_serial_port: bool) -> String {
    if !_native_serial_port && is_wsl() {
        "COM3".to_string()
    } else {
        "/dev/ttyUSB0".to_string()
    }
}

// Check the target folder is valid
pub fn check_target_folder_valid(target_folder: &str, clean: bool) -> bool{
    // Check the target folder exists
    if !Path::new(&target_folder).exists() {
        // Create the folder if possible
        match std::fs::create_dir(&target_folder) {
            Ok(_) => println!("Created folder: {}", target_folder),
            Err(e) => {
                println!("Error creating folder: {}", e);
                return false;
            }
        }
    } else {
        // Check the folder is empty
        if std::fs::read_dir(&target_folder).unwrap().next().is_some() {
            if clean {
                // Delete the contents of the folder
                match remove_dir_contents(&target_folder) {
                    Ok(_) => println!("Deleted folder contents: {}", target_folder),
                    Err(e) => {
                        println!("Error deleting folder contents: {}", e);
                        return false;
                    }
                }
            } else {
                println!("Error: target folder must be empty: {}", target_folder);
                return false;
            }
        }
    }
    true
}
