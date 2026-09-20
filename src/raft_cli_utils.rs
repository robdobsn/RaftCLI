use std::collections::HashMap;
#[cfg(not(target_os = "windows"))]
use std::env;
use std::path::Path;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::fs;
use std::error::Error;
// use regex::Regex;
use std::fmt::{self, Display, Formatter};
use std::io::{self, BufRead, BufReader};
use std::sync::{Arc, Mutex};
use remove_dir_all::remove_dir_contents;
use crossbeam::thread;

pub fn default_esp_idf_version() -> String {
    // Default ESP-IDF version
    "6.0.2".to_string()
}

// Build information structure
#[derive(Debug, Clone)]
pub struct BuildInfo {
    pub last_built_systype: Option<String>,
    pub last_build_method: Option<String>,  // "docker" or "local_idf"
    pub last_idf_path_explicit: bool,
    pub last_idf_path: Option<String>,
    pub last_port: Option<String>,
    pub last_monitor_baud: Option<u32>,
    pub last_flash_baud: Option<u32>,
    pub last_vid: Option<String>,
    pub last_no_fs: Option<bool>,
    pub last_ip_addr: Option<String>,
    // ESP-IDF version last used to build each SysType (a build folder can't be reused with a different version)
    pub idf_versions: HashMap<String, String>,
}

impl Default for BuildInfo {
    fn default() -> Self {
        BuildInfo {
            last_built_systype: None,
            last_build_method: None,
            last_idf_path_explicit: false,
            last_idf_path: None,
            last_port: None,
            last_monitor_baud: None,
            last_flash_baud: None,
            last_vid: None,
            last_no_fs: None,
            last_ip_addr: None,
            idf_versions: HashMap::new(),
        }
    }
}

// Read build information from raft.info file
pub fn read_build_info(app_folder: &str) -> BuildInfo {
    let raft_info_path = format!("{}/build/raft.info", app_folder);
    if let Ok(contents) = fs::read_to_string(&raft_info_path) {
        // Parse JSON to extract build info
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&contents) {
            return BuildInfo {
                last_built_systype: json["last_built_systype"].as_str().map(|s| s.to_string()),
                last_build_method: json["last_build_method"].as_str().map(|s| s.to_string()),
                last_idf_path_explicit: json["last_idf_path_explicit"].as_bool().unwrap_or(false),
                last_idf_path: json["last_idf_path"].as_str().map(|s| s.to_string()),
                last_port: json["last_port"].as_str().map(|s| s.to_string()),
                last_monitor_baud: json["last_monitor_baud"].as_u64().map(|v| v as u32),
                last_flash_baud: json["last_flash_baud"].as_u64().map(|v| v as u32),
                last_vid: json["last_vid"].as_str().map(|s| s.to_string()),
                last_no_fs: json["last_no_fs"].as_bool(),
                last_ip_addr: json["last_ip_addr"].as_str().map(|s| s.to_string()),
                idf_versions: json["idf_versions"].as_object().map(|versions| {
                    versions.iter()
                        .filter_map(|(sys_type, version)| version.as_str().map(|v| (sys_type.clone(), v.to_string())))
                        .collect()
                }).unwrap_or_default(),
            };
        }
    }
    BuildInfo::default()
}

// Write build information to raft.info file using merge semantics
// Only fields that are Some will be updated; existing values are preserved
pub fn write_build_info(
    app_folder: &str,
    updates: &BuildInfo,
) -> Result<(), Box<dyn std::error::Error>> {
    let build_folder = format!("{}/build", app_folder);
    
    // Create build folder if it doesn't exist
    if !Path::new(&build_folder).exists() {
        fs::create_dir_all(&build_folder)?;
    }
    
    let raft_info_path = format!("{}/raft.info", build_folder);

    // Read existing info to merge with
    let existing = read_build_info(app_folder);

    // Merge: updates take priority over existing values
    let merged = BuildInfo {
        last_built_systype: updates.last_built_systype.clone().or(existing.last_built_systype),
        last_build_method: updates.last_build_method.clone().or(existing.last_build_method),
        last_idf_path_explicit: if updates.last_idf_path.is_some() { updates.last_idf_path_explicit } else { existing.last_idf_path_explicit },
        last_idf_path: updates.last_idf_path.clone().or(existing.last_idf_path),
        last_port: updates.last_port.clone().or(existing.last_port),
        last_monitor_baud: updates.last_monitor_baud.or(existing.last_monitor_baud),
        last_flash_baud: updates.last_flash_baud.or(existing.last_flash_baud),
        last_vid: updates.last_vid.clone().or(existing.last_vid),
        last_no_fs: updates.last_no_fs.or(existing.last_no_fs),
        last_ip_addr: updates.last_ip_addr.clone().or(existing.last_ip_addr),
        idf_versions: {
            let mut idf_versions = existing.idf_versions;
            idf_versions.extend(updates.idf_versions.clone());
            idf_versions
        },
    };

    let mut raft_info = serde_json::json!({
        "last_idf_path_explicit": merged.last_idf_path_explicit
    });

    if let Some(ref v) = merged.last_built_systype { raft_info["last_built_systype"] = serde_json::json!(v); }
    if let Some(ref v) = merged.last_build_method { raft_info["last_build_method"] = serde_json::json!(v); }
    if let Some(ref v) = merged.last_idf_path { raft_info["last_idf_path"] = serde_json::json!(v); }
    if let Some(ref v) = merged.last_port { raft_info["last_port"] = serde_json::json!(v); }
    if let Some(v) = merged.last_monitor_baud { raft_info["last_monitor_baud"] = serde_json::json!(v); }
    if let Some(v) = merged.last_flash_baud { raft_info["last_flash_baud"] = serde_json::json!(v); }
    if let Some(ref v) = merged.last_vid { raft_info["last_vid"] = serde_json::json!(v); }
    if let Some(v) = merged.last_no_fs { raft_info["last_no_fs"] = serde_json::json!(v); }
    if let Some(ref v) = merged.last_ip_addr { raft_info["last_ip_addr"] = serde_json::json!(v); }
    if !merged.idf_versions.is_empty() { raft_info["idf_versions"] = serde_json::json!(merged.idf_versions); }
    
    fs::write(&raft_info_path, serde_json::to_string_pretty(&raft_info)?)?;
    Ok(())
}

pub fn utils_get_sys_type(
    build_sys_type: &Option<String>, 
    app_folder: String
) -> Result<String, Box<dyn std::error::Error>> {
    // Determine the Systype to build - priority order:
    // 1. SysType passed in via -s flag
    // 2. Last built systype from raft.info file
    // 3. First SysType found in the systypes folder (excluding Common)
    let mut sys_type: String = String::new();
    if let Some(build_sys_type) = build_sys_type {
        sys_type = build_sys_type.to_string();
    } else {
        // Try to read from raft.info file
        let build_info = read_build_info(&app_folder);
        if let Some(last_systype) = build_info.last_built_systype {
            // Verify the systype still exists in the systypes folder
            let systype_path = format!("{}/{}/{}", app_folder, get_systypes_folder_name(), last_systype);
            if Path::new(&systype_path).exists() {
                sys_type = last_systype;
            }
        }
        
        // If still no systype, fall back to first non-Common folder
        if sys_type.is_empty() {
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
    }

    Ok(sys_type)
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

pub fn execute_and_capture_output(command: String, args: &Vec<String>, cur_dir: String, env_vars_to_add: HashMap<String, String>) -> Result<(String, bool), CommandError> {
    execute_and_capture_output_env(command, args, cur_dir, env_vars_to_add, &vec![])
}

pub fn execute_and_capture_output_env(command: String, args: &Vec<String>, cur_dir: String, env_vars_to_add: HashMap<String, String>,
            env_vars_to_remove: &Vec<String>) -> Result<(String, bool), CommandError> {

    let mut command_builder = Command::new(command.clone());
    for name in env_vars_to_remove {
        command_builder.env_remove(name);
    }
    let process = command_builder
        .current_dir(cur_dir)
        .args(args)
        .envs(env_vars_to_add.iter())
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

// Check if esptool can be run via Python module
fn check_python_esptool() -> bool {
    Command::new("python")
        .args(&["-m", "esptool", "version"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

pub fn get_flash_tool_cmd(flash_tool_opt: Option<String>, native_serial_port: bool) -> String {
    match flash_tool_opt {
        Some(tool) => tool,
        None => {
            let possible_executables = if cfg!(target_os = "windows") {
                // On Windows, esptool installed via pip is typically just "esptool" (handled by Python Scripts)
                vec!["esptool", "esptool.py", "esptool.exe"]
            } else if is_wsl() {
                if native_serial_port {
                    vec!["esptool.py", "esptool"]
                } else {
                    // When delegating to Windows from WSL, try esptool first (Python-installed version)
                    vec!["esptool", "esptool.py.exe", "esptool.exe"]
                }
            } else {
                vec!["esptool.py", "esptool"]
            };

            if let Some(exe) = find_executable(&possible_executables) {
                exe
            } else if check_python_esptool() {
                // If esptool is available as a Python module, use that
                "python -m esptool".to_string()
            } else {
                // Fallback to default if not found
                if cfg!(target_os = "windows") {
                    "esptool".to_string()
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

// pub fn get_device_type(sys_type: String, app_folder: String) -> String {
//     // Get build folder
//     let build_folder = get_build_folder_name(sys_type, app_folder);

//     // Read the project_description.json file
//     let project_description = fs::read_to_string(format!("{}/project_description.json", build_folder));

//     // Check for errors reading the project_description.json file
//     if project_description.is_err() {
//         println!("Error reading the project_description.json file: {}", project_description.err().unwrap());
//         return "esp32".to_string();
//     }

//     // Extract the device type from the project_description.json file
//     let project_description = project_description.unwrap();
//     let device_type_regex = Regex::new(r#""target":\s*"([^"]+)""#).unwrap();
//     let device_type = device_type_regex.captures(&project_description);

//     // Check for errors extracting the device type
//     if device_type.is_none() {
//         println!("Error extracting the device type from the project_description.json file");
//         return "esp32".to_string();
//     }

//     // Return the device type
//     device_type.unwrap()[1].to_string()
// }

pub fn build_flash_command_args(
    build_folder: String,
    port: &str,
    flash_baud: u32,
    skip_fs: bool,
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    // Flash arguments file
    let flash_args_file = format!("{}/flasher_args.json", build_folder);

    // Read the flash arguments json file
    let flash_args = fs::read_to_string(&flash_args_file)?;

    // Extract the flash arguments
    let flash_args: serde_json::Value = serde_json::from_str(&flash_args)?;

    // Flash baud string
    let flash_baud = format!("{}", flash_baud);

    // Extract flash settings
    let flash_mode = flash_args["flash_settings"]["flash_mode"].as_str().unwrap();
    let flash_size = flash_args["flash_settings"]["flash_size"].as_str().unwrap();
    let flash_freq = flash_args["flash_settings"]["flash_freq"].as_str().unwrap();
    let chip_type = flash_args["extra_esptool_args"]["chip"].as_str().unwrap();

    // Create initial esptool arguments
    let mut esptool_args = vec![
        "-p".to_string(),
        port.to_string(),
        "-b".to_string(),
        flash_baud,
        "--before".to_string(),
        "default_reset".to_string(),
        "--after".to_string(),
        "hard_reset".to_string(),
        "--chip".to_string(),
        chip_type.to_string(),
        "write_flash".to_string(),
        "--flash_mode".to_string(),
        flash_mode.to_string(),
        "--flash_size".to_string(),
        flash_size.to_string(),
        "--flash_freq".to_string(),
        flash_freq.to_string(),
    ];

    // Collect known non-FS offsets (bootloader, app, partition-table) from flasher_args.json
    let mut known_offsets: Vec<String> = Vec::new();
    for key in &["bootloader", "app", "partition-table", "partition_table"] {
        if let Some(offset) = flash_args[key]["offset"].as_str() {
            known_offsets.push(offset.to_string());
        }
    }

    // Extract and append flash files and their offsets
    if let Some(flash_files) = flash_args["flash_files"].as_object() {
        for (offset, file_path) in flash_files {
            let file_path_str = file_path.as_str().unwrap();

            // Skip filesystem entries if requested
            if skip_fs && !known_offsets.contains(offset) {
                let lower = file_path_str.to_lowercase();
                let basename = lower.rsplit(|c| c == '/' || c == '\\').next().unwrap_or(&lower);
                if basename == "fs.bin"
                    || lower.contains("spiffs") || lower.contains("littlefs") 
                    || lower.contains("fatfs") || lower.contains("storage")
                    || lower.contains("fs_image") {
                    println!("Skipping filesystem image: {}", file_path_str);
                    continue;
                }
            }

            let full_path = format!("{}/{}", build_folder, file_path_str);
            esptool_args.push(offset.clone());
            esptool_args.push(full_path);
        }
    }

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

// Function to check if Docker is available
pub fn is_docker_available() -> bool {
    Command::new("docker")
        .arg("--version")
        .output()
        .map_or(false, |output| output.status.success())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raft_info_is_backward_compatible_and_records_idf_versions() {
        let app_folder = std::env::temp_dir().join(format!("raftcli_info_test_{}", std::process::id()));
        let _ = fs::remove_dir_all(&app_folder);
        fs::create_dir_all(app_folder.join("build")).unwrap();
        let app_folder_str = app_folder.to_string_lossy().to_string();

        // A raft.info written by an earlier version of RaftCLI (no idf_versions)
        fs::write(app_folder.join("build").join("raft.info"),
            r#"{"last_built_systype":"BoardA","last_build_method":"local_idf","last_idf_path_explicit":true,"last_idf_path":"/home/me/esp/esp-idf-v6.0.2","last_port":"COM15"}"#).unwrap();
        let info = read_build_info(&app_folder_str);
        assert_eq!(info.last_built_systype.as_deref(), Some("BoardA"));
        assert!(info.last_idf_path_explicit);
        assert!(info.idf_versions.is_empty());

        // Versions are recorded per SysType and merged with what is already there
        for (sys_type, version) in [("BoardA", "6.0.2"), ("BoardB", "6.1.0"), ("BoardA", "6.1.0")] {
            let mut updates = BuildInfo::default();
            updates.idf_versions.insert(sys_type.to_string(), version.to_string());
            write_build_info(&app_folder_str, &updates).unwrap();
        }
        let info = read_build_info(&app_folder_str);
        assert_eq!(info.idf_versions.get("BoardA").map(|s| s.as_str()), Some("6.1.0"));
        assert_eq!(info.idf_versions.get("BoardB").map(|s| s.as_str()), Some("6.1.0"));
        assert_eq!(info.last_port.as_deref(), Some("COM15"));
        assert_eq!(info.last_idf_path.as_deref(), Some("/home/me/esp/esp-idf-v6.0.2"));

        let _ = fs::remove_dir_all(&app_folder);
    }
}
