use std::path::Path;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::fs;
use std::io::{self, BufRead, BufReader};
use std::sync::{Arc, Mutex};
use crossbeam::thread;

pub fn utils_get_sys_type(build_sys_type: &Option<String>, app_folder: &str,
            sys_types_base_folder_rel: &str) -> String {

    // Determine the Systype to build - this is either the SysType passed in or
    // the first SysType found in the systypes folder (excluding Common)
    let mut sys_type: String = String::new();
    if let Some(build_sys_type) = build_sys_type {
        sys_type = build_sys_type.to_string();
    } else {
        let sys_types = fs::read_dir(
            format!("{}/{}", app_folder, sys_types_base_folder_rel)
        );
        if sys_types.is_err() {
            println!("Error reading the systypes folder: {}", sys_types.err().unwrap());
            std::process::exit(1);
        }
        for sys_type_dir_entry in sys_types.unwrap() {
            let sys_type_dir = sys_type_dir_entry;
            if sys_type_dir.is_err() {
                println!("Error reading the systypes folder: {}", sys_type_dir.err().unwrap());
                std::process::exit(1);
            }
            let sys_type_name = sys_type_dir.unwrap().file_name().into_string().unwrap();
            if sys_type_name != "Common" {
                sys_type = sys_type_name;
                break;
            }
        }
    }

    sys_type
}

// Check the app folder is valid
pub fn check_app_folder_valid(app_folder: &str, sys_types_base_folder_rel: &str) {
    // The app folder is valid if it exists and contains a CMakeLists.txt file
    // and a folder called systypes 
    let cmake_file = format!("{}/CMakeLists.txt", app_folder);
    if !Path::new(&app_folder).exists() {
        println!("Error: app folder does not exist: {}", app_folder);
        std::process::exit(1);
    } else if !Path::new(&cmake_file).exists() {
        println!("Error: app folder does not contain a CMakeLists.txt file: {}", app_folder);
        std::process::exit(1);
    } else if !Path::new(&format!("{}/{}", app_folder, sys_types_base_folder_rel)).exists() {
        println!("Error: app folder does not contain a systypes folder: {}", app_folder);
        std::process::exit(1);
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

pub fn execute_and_capture_output(command: &str, args: &[&str], cur_dir: &str) -> io::Result<String> {
    let process = Command::new(command)
        .current_dir(cur_dir)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let stdout = process.stdout.expect("failed to capture stdout");
    let stderr = process.stderr.expect("failed to capture stderr");

    let stdout_reader = BufReader::new(stdout);
    let stderr_reader = BufReader::new(stderr);

    let captured_output = Arc::new(Mutex::new(String::new()));

    // Using crossbeam to handle threads easily
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