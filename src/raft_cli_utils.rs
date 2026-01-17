use std::collections::HashMap;
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
    "5.5.2".to_string()
}

// Build information structure
#[derive(Debug, Clone)]
pub struct BuildInfo {
    pub last_built_systype: Option<String>,
    pub last_build_method: Option<String>,  // "docker" or "local_idf"
    pub last_idf_path_explicit: bool,
    pub last_idf_path: Option<String>,
}

impl Default for BuildInfo {
    fn default() -> Self {
        BuildInfo {
            last_built_systype: None,
            last_build_method: None,
            last_idf_path_explicit: false,
            last_idf_path: None,
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
            };
        }
    }
    BuildInfo::default()
}

// Write build information to raft.info file
pub fn write_build_info(
    app_folder: &str,
    sys_type: &str,
    build_method: &str,
    idf_path_explicit: bool,
    idf_path: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let build_folder = format!("{}/build", app_folder);
    
    // Create build folder if it doesn't exist
    if !Path::new(&build_folder).exists() {
        fs::create_dir_all(&build_folder)?;
    }
    
    let raft_info_path = format!("{}/raft.info", build_folder);
    let mut raft_info = serde_json::json!({
        "last_built_systype": sys_type,
        "last_build_method": build_method,
        "last_idf_path_explicit": idf_path_explicit
    });
    
    // Add idf_path if present
    if let Some(path) = idf_path {
        raft_info["last_idf_path"] = serde_json::json!(path);
    }
    
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
    
    let process = Command::new(command.clone())
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

    // Extract and append flash files and their offsets
    if let Some(flash_files) = flash_args["flash_files"].as_object() {
        for (offset, file_path) in flash_files {
            let full_path = format!("{}/{}", build_folder, file_path.as_str().unwrap());
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

// Check if ESP IDF Environment is active
pub fn is_esp_idf_env() -> bool {
    // Check if the IDF_PATH environment variable is set
    env::var("IDF_PATH").is_ok()
}

// Check if the ESP IDF version is correct
pub fn idf_version_ok(required_esp_idf_version: String) -> bool {
    // Run the idf.py --version command
    let idf_output = Command::new("idf.py")
        .arg("--version")
        .output()
        .expect("Failed to run idf.py --version");

    // TODO remove
    println!("idf_version returned from idf.py: {:?}", idf_output);

    // Check if the command was successful
    if !idf_output.status.success() {
        println!("Failed to run idf.py --version");
        return false;
    }

    // Extract the version string from the output
    let idf_version_output = String::from_utf8_lossy(&idf_output.stdout);
    let idf_version = idf_version_output
        .split_whitespace() // Split by whitespace
        .nth(1)             // Get the second token (e.g., "v5.3.1-dirty")
        .unwrap_or("")      // Fallback to an empty string if parsing fails
        .trim_start_matches('v') // Remove the leading 'v' if present
        .split('-')         // Split by '-' to ignore any suffix like '-dirty'
        .next()             // Take the first part (e.g., "5.3.1")
        .unwrap_or("");

    // Normalize both versions to major.minor.patch format
    let idf_version_normalized = idf_version.split('.').take(3).collect::<Vec<&str>>().join(".");
    let required_version_normalized = required_esp_idf_version.split('.').take(3).collect::<Vec<&str>>().join(".");

    // Debugging: Print normalized versions
    println!(
        "idf_version_normalized: {:?}, required_version_normalized: {:?}",
        idf_version_normalized, required_version_normalized
    );

    // Compare the normalized versions
    if idf_version_normalized != required_version_normalized {
        println!(
            "Error: ESP-IDF version mismatch: Required: {}, Found: {}",
            required_version_normalized, idf_version_normalized
        );
        return false;
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

pub fn get_esp_idf_version_from_dockerfile(dockerfile_path: &str) -> Result<String, Box<dyn std::error::Error>> {
    let dockerfile_path = Path::new(dockerfile_path).join("Dockerfile");
    let dockerfile_content = fs::read_to_string(dockerfile_path)?;
    for line in dockerfile_content.lines() {
        if line.starts_with("FROM espressif/idf:") {
            let version = line.replace("FROM espressif/idf:", "").trim().to_string();
            // Remove the 'v' prefix if it exists
            if version.starts_with('v') {
                return Ok(version[1..].to_string());
            }
            return Ok(version);
        }
    }
    Err(Box::new(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        "ESP-IDF version not found in Dockerfile",
    )))
}

pub fn find_matching_esp_idf(target_version: String, user_path: Option<String>) -> Option<PathBuf> {
    // 1. Check user-specified path
    if let Some(path) = user_path {
        let user_dir = Path::new(&path);
        if user_dir.is_dir() {
            // Check if the folder is an ESP-IDF folder by checking if it contains a file named export.sh
            if user_dir.join("export.sh").is_file() {
                // TODO remove
                println!("Found required ESP IDF folder {:?}", user_dir);
                return Some(user_dir.to_path_buf());
            }
            // If it's a directory, look for subfolders named esp-idf-vx.y.z
            if let Some(matching_path) = user_dir
                .read_dir()
                .ok()?
                .filter_map(|entry| entry.ok())
                .map(|entry| entry.path())
                .find(|p| p.file_name().map_or(false, |name| name.to_string_lossy().ends_with(&target_version)))
            {
                // TODO remove
                println!("Found matching path: {:?}", matching_path);
                return Some(matching_path);
            }
        }
    }

    // 2. Default paths based on the platform
    let default_paths = get_default_esp_idf_paths();

    // TODO remove
    println!("Searching default paths: {:?}", default_paths);

    for path in default_paths {
        if path.is_dir() {
            if let Some(matching_path) = path
                .read_dir()
                .ok()?
                .filter_map(|entry| entry.ok())
                .map(|entry| entry.path())
                .find(|p| p.file_name().map_or(false, |name| name.to_string_lossy().ends_with(&target_version)))
            {
                // TODO remove
                println!("Found matching path: {:?}", matching_path);
                return Some(matching_path);
            }
        }
    }

    // TODO remove
    println!("No matching ESP-IDF found for {:?}", target_version);
    None
}

// Helper function to get default paths based on OS
fn get_default_esp_idf_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();

    #[cfg(target_os = "linux")]
    paths.push(dirs::home_dir().unwrap_or_default().join("esp"));

    #[cfg(target_os = "windows")]
    paths.push(PathBuf::from("C:\\Espressif\\frameworks"));

    #[cfg(target_os = "macos")]
    paths.push(dirs::home_dir().unwrap_or_default().join("esp"));

    paths
}

pub fn prepare_esp_idf(idf_path: &Path) -> Result<HashMap<String, String>, Box<dyn std::error::Error>> {
    let mut env_vars = HashMap::new();

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        let export_script = idf_path.join("export.sh");
        if export_script.exists() {
            println!("Capturing ESP-IDF environment from {}", idf_path.display());
            let output = Command::new("bash")
                .arg("-c")
                .arg(format!("source {} && env", export_script.display()))
                .stdout(Stdio::piped())
                .output()?;
            if !output.status.success() {
                return Err(Box::new(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "Failed to capture ESP-IDF environment",
                )));
            }

            // Parse the environment variables
            for line in String::from_utf8_lossy(&output.stdout).lines() {
                if let Some((key, value)) = line.split_once('=') {
                    env_vars.insert(key.to_string(), value.to_string());
                }
            }
        } else {
            return Err(Box::new(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "export.sh not found in ESP-IDF folder",
            )));
        }
    }

    #[cfg(target_os = "windows")]
    {
        let export_script = idf_path.join("export.bat");
        if export_script.exists() {
            println!("Capturing ESP-IDF environment from {}", idf_path.display());
            let output = Command::new("cmd")
                .args(["/C", export_script.to_str().unwrap(), "&&", "set"])
                .stdout(Stdio::piped())
                .output()?;
            if !output.status.success() {
                return Err(Box::new(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "Failed to capture ESP-IDF environment",
                )));
            }

            // Parse the environment variables
            for line in String::from_utf8_lossy(&output.stdout).lines() {
                if let Some((key, value)) = line.split_once('=') {
                    env_vars.insert(key.to_string(), value.to_string());
                }
            }
        } else {
            return Err(Box::new(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "export.bat not found in ESP-IDF folder",
            )));
        }
    }

    Ok(env_vars)
}
