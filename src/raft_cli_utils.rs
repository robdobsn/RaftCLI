use std::path::Path;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::fs;
use std::io::{self, BufRead, BufReader};
use std::sync::{Arc, Mutex};
use remove_dir_all::remove_dir_contents;
use crossbeam::thread;

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

pub fn execute_and_capture_output(command: &str, args: &Vec<String>, cur_dir: &str) -> io::Result<String> {

    let process = Command::new(command)
        .current_dir(cur_dir)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    if process.is_err() {
        println!("Error executing command: {}", &process.as_ref().err().unwrap());
        return Err(process.err().unwrap());
    }

    let mut process = process.unwrap();
    let stdout = process.stdout.take().unwrap();
    let stderr = process.stderr.take().unwrap();

    let stdout_reader = BufReader::new(stdout);
    let stderr_reader = BufReader::new(stderr);

    let captured_output = Arc::new(Mutex::new(String::new()));

    // Using crossbeam to handle threads
    thread::scope(|s| {
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
    }).unwrap();
    
    let output = captured_output.lock().unwrap().clone();
    Ok(output)
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
            return proc_version.unwrap().contains("Microsoft") || proc_version.unwrap().contains("WSL");
        }
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
    // Extract the command to flash the app using the esptool.py as the keyword to locate the start of the command
    let flash_command_start = output.find("-p (PORT)");
    let mut flash_command: String;
    match flash_command_start {
        Some(start) => {
            flash_command = output[start..].to_string();
            // Truncase the string at the first newline character
            let flash_command_end = flash_command.find("\n");
            if !flash_command_end.is_none() {
                flash_command.truncate(flash_command_end.unwrap());
            }
        },
        None => {
            return Err(Box::new(std::io::Error::new(std::io::ErrorKind::Other, "Flash command not found in output")));
        }
    }

    // The required arguments for flashing the app will look something like this
    // -p {{port}} -b {{flash_baud}} --before default_reset --after hard_reset --chip esp32  write_flash --flash_mode dio --flash_size 4MB --flash_freq 40m 0x1000 build/SysTypeMain/bootloader/bootloader.bin 0x8000 build/SysTypeMain/partition_table/partition-table.bin 0x1e000 build/SysTypeMain/ota_data_initial.bin 0x20000 build/SysTypeMain/SysTypeMain.bin 0x380000 build/SysTypeMain/fs.bin

    // Replace the placeholders with the actual values
    let flash_command = flash_command
                                    .replace("(PORT)", port)
                                    .replace("-b 460800", &format!("-b {}", flash_baud.to_string()));

    // Create a vector by splitting the string on whitespace
    let flash_command_parts: Vec<String> = flash_command.split_whitespace().map(|s| s.to_string()).collect();

    // Debug
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
