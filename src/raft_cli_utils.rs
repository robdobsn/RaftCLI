// raft_cli_utils.rs - RaftCLI: Utilities
// Rob Dobson 2024

use std::env;
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
use ini::Ini;

/// @brief Get a list of SysTypes
/// @param build_sys_type The SysType if specified on the command line
/// @param app_folder The app folder
/// @return List of SysTypes
/// @note the returned value is a list containing (a) the single SysType passed in (from command line) OR
///       (b) if platform.ini is present and there is a default_envs entry in the raft section then that list OR
///       (c) if platform.ini is present and there are env:: entries then the first one of those OR
///       (d) if none of the above then the first SysType found in the systypes folder (excluding Common)
pub fn utils_get_sys_type_list(
    build_sys_type: &Option<String>, 
    app_folder: String
) -> Result<Vec<String>, Box<dyn std::error::Error>> {

    // Get the list of SysTypes
    let mut sys_types: Vec<String> = Vec::new();

    if let Some(build_sys_type) = build_sys_type {
        sys_types.push(build_sys_type.to_string());
    } else {
        // Read a platform.ini file if it exists
        let platform_ini = read_platform_ini(app_folder.clone());

        // If platform.ini exists, process the default_envs field
        if let Ok(ref platform_ini) = platform_ini {
            if let Some(default_envs) = platform_ini.get_from(Some("raft"), "default_envs") {
                let envs: Vec<String> = default_envs.split(',')
                    .map(|s| s.trim().to_string()) // Trim and convert to String
                    .collect();

                if !envs.is_empty() {
                    sys_types.extend(envs); // Add all the environments to the list
                }
            }
        }

        // Check if the list of SysTypes is empty
        if sys_types.is_empty() {

            let sys_type_sections: Vec<String> = platform_ini.unwrap().sections()
            .filter_map(|s| {
                if s?.starts_with("env::") {
                    Some(s?.trim_start_matches("env::").to_string())
                } else {
                    None
                }
            })
            .collect();
    
            // If there is a valid list of SysTypes in the platform.ini file then use the first one in that list
            if !sys_type_sections.is_empty() {
                sys_types.push(sys_type_sections[0].clone());
            }

        } else {
            // Get the list of SysTypes found in the systypes folder (excluding Common)
            let sys_types_dir = fs::read_dir(
                format!("{}/{}", app_folder, get_systypes_folder_name())
            );
            if sys_types_dir.is_err() {
                println!("Error reading the systypes folder: {}", sys_types_dir.err().unwrap());
                return Err(Box::new(std::io::Error::new(std::io::ErrorKind::Other, "Error reading the systypes folder")));
            }

            let mut sys_type_dir_list: Vec<String> = Vec::new();
            for sys_type_dir_entry in sys_types_dir.unwrap() {
                let sys_type_dir = sys_type_dir_entry;
                if sys_type_dir.is_err() {
                    println!("Error reading the systypes folder: {}", sys_type_dir.err().unwrap());
                    return Err(Box::new(std::io::Error::new(std::io::ErrorKind::Other, "Error reading the systypes folder")));
                }
                let sys_type_name = sys_type_dir.unwrap().file_name().into_string().unwrap();
                if sys_type_name != "Common" {
                    sys_type_dir_list.push(sys_type_name);
                }
            }

            // If the list isn't empty then use the first element in it
            if !sys_type_dir_list.is_empty() {
                sys_types.push(sys_type_dir_list[0].clone());
            }
        }
    }

    Ok(sys_types)
}

// Check the app folder is valid
pub fn check_app_folder_valid(app_folder: String) -> bool {
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

pub fn check_for_raft_artifacts_deletion(app_folder: String, sys_type: String) -> bool {
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
        } else {
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

pub fn execute_and_capture_output(command: String, args: &Vec<String>, cur_dir: String) -> Result<(String, bool), CommandError> {
    let process = Command::new(command.clone())
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
                return Err(CommandError::CommandNotFound(format!("{}: No such file or directory", command.clone())));
            } else {
                return Err(CommandError::Other(e));
            }
        }
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

pub fn find_executable(executables: &[&str]) -> Option<String> {
    // println!("executables: {:?}", executables);
    for &exe in executables {
        if which::which(exe).is_ok() {
            // println!("exe ok: {:?}", exe);
            return Some(exe.to_string());
        }
    }
    None
}

pub fn get_flash_tool_cmd(flash_tool_opt: Option<String>, native_serial_port: bool) -> String {
    match flash_tool_opt {
        Some(tool) => tool,
        None => {
            let possible_executables = if cfg!(target_os = "windows") {
                vec!["esptool.py.exe", "esptool.exe"]
            } else if is_wsl() {
                if native_serial_port {
                    vec!["esptool.py", "esptool"]
                } else {
                    vec!["esptool.py.exe", "esptool.exe"]
                }
            } else {
                vec!["esptool.py", "esptool"]
            };

            if let Some(exe) = find_executable(&possible_executables) {
                exe
            } else {
                // Fallback to default if not found
                if cfg!(target_os = "windows") {
                    "esptool.exe".to_string()
                } else {
                    "esptool.py".to_string()
                }
            }
        }
    }
}

pub fn get_build_folder_name(sys_type: String, app_folder: String) -> String {
    let build_folder_name = format!("{}/build/{}", app_folder, sys_type);
    build_folder_name
}

pub fn get_device_type(sys_type: String, app_folder: String) -> String {
    // Get build folder
    let build_folder = get_build_folder_name(sys_type, app_folder);

    // Read the project_description.json file
    let project_description = fs::read_to_string(format!("{}/project_description.json", build_folder));

    // Check for errors reading the project_description.json file
    if project_description.is_err() {
        println!("Error reading the project_description.json file: {}", project_description.err().unwrap());
        return "esp32".to_string();
    }

    // Extract the device type from the project_description.json file
    let project_description = project_description.unwrap();
    let device_type_regex = Regex::new(r#""target":\s*"([^"]+)""#).unwrap();
    let device_type = device_type_regex.captures(&project_description);

    // Check for errors extracting the device type
    if device_type.is_none() {
        println!("Error extracting the device type from the project_description.json file");
        return "esp32".to_string();
    }

    // Return the device type
    device_type.unwrap()[1].to_string()
}

pub fn extract_flash_cmd_args(
    _output: String,
    device_type: String,
    port: &str,
    flash_baud: u32,
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    // Flash baud string
    let flash_baud = format!("{}", flash_baud);

    // Flash command parts
    let esptool_args = vec![
        "-p", port,
        "-b", flash_baud.as_str(),
        "--before", "default_reset",
        "--after", "hard_reset",
        "--chip", &device_type,
        "write_flash",
        "@flash_args",
    ];

    // Create a vector of strings from esptool_args
    let esptool_args: Vec<String> = esptool_args.iter().map(|s| s.to_string()).collect();

    Ok(esptool_args)
}

// Check the target folder is valid
pub fn check_target_folder_valid(target_folder: &str, clean: bool) -> bool {
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

// Check if ESP IDF Environment is active
pub fn is_esp_idf_env() -> bool {
    // Check if the IDF_PATH environment variable is set
    env::var("IDF_PATH").is_ok()
}

// Function to check if Docker is available
pub fn is_docker_available() -> bool {
    Command::new("docker")
        .arg("--version")
        .output()
        .map_or(false, |output| output.status.success())
}


pub fn read_platform_ini(project_dir: String) -> Result<Ini, Box<dyn std::error::Error>> {
    let platform_ini_path = format!("{}/platform.ini", project_dir);
    if Path::new(&platform_ini_path).exists() {
        let conf = Ini::load_from_file(platform_ini_path)?;
        Ok(conf)
    } else {
        Err(Box::new(std::io::Error::new(std::io::ErrorKind::NotFound, "platform.ini not found")))
    }
}
