
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};
use std::path::PathBuf;
use std::io::{self, BufRead, BufReader};
use std::sync::{Arc, Mutex};
use crossbeam::thread;

pub fn build_raft_app(build_sys_type: Option<String>, clean: bool, app_folder: String,
            no_docker_arg: bool, idf_path_full: Option<String>) 
                            -> Result<(), Box<dyn std::error::Error>> {

    // systypes folder name
    let sys_types_base_folder_rel = "systypes";

    // Check the app folder is valid
    check_app_folder_valid(&app_folder, sys_types_base_folder_rel);

    // Determine the Systype to build - this is either the SysType passed in or
    // the first SysType found in the systypes folder (excluding Common)
    let mut sys_type: String = String::new();
    if let Some(build_sys_type) = build_sys_type {
        sys_type = build_sys_type;
    } else {
        let sys_types = fs::read_dir(
            format!("{}/{}", app_folder, sys_types_base_folder_rel)
        )?;
        for sys_type_dir_entry in sys_types {
            let sys_type_dir = sys_type_dir_entry?;
            let sys_type_name = sys_type_dir.file_name().into_string().unwrap();
            if sys_type_name != "Common" {
                sys_type = sys_type_name;
                break;
            }
        }
    }

    // Flags indicating the build folder and "build_raft_artifacts" folder should be deleted
    let mut delete_build_folder = false;
    let mut delete_build_raft_artifacts_folder = false;

    // If clean is true, delete the build folder for the SysType to built and
    // the "build_raft_artifacts" folder
    if clean {
        delete_build_folder = true;
        delete_build_raft_artifacts_folder = true;
    } else {
        // Check if the "build_raft_artifacts" folder exists inside the app folder
        // and if so extract the contents of the "cursystype.txt" file to determine
        // the SysType of the last build - then check if this is the same as the
        // sys_type to build and if not, delete the "build_raft_artifacts" folder
        let build_raft_artifacts_folder = format!("{}/build_raft_artifacts", app_folder);
        if Path::new(&build_raft_artifacts_folder).exists() {
            let cursystype_file = format!("{}/cursystype.txt", build_raft_artifacts_folder);
            if Path::new(&cursystype_file).exists() {
                let cursystype = fs::read_to_string(&cursystype_file)?;
                if cursystype.trim() != sys_type {
                    println!("Delete the build_raft_artifacts folder as the SysType to build has changed");
                    delete_build_raft_artifacts_folder = true;
                }
            }
            else
            {
                println!("Delete the build_raft_artifacts folder as the cursystype.txt file is missing");
                delete_build_raft_artifacts_folder = true;
            }
        }
    }

    // Build the app
    println!("Building the SysType {} app in folder: {} delete_build {} delete_raft_artifacts {}", 
                        sys_type, app_folder, delete_build_folder, delete_build_raft_artifacts_folder);

    // Determine if docker is to be used for build
    let mut no_docker = std::env::var("RAFT_NO_DOCKER").unwrap_or("false".to_string()) == "true";
    if no_docker_arg {
        no_docker = true;
    }

    // Handle building with or without docker
    if no_docker {
        // Get idf path
        let idf_path = idf_path_full.unwrap_or("idf.py".to_string());

        // Build without docker
        let build_result = build_without_docker(&app_folder, &sys_type, clean, 
                    delete_build_folder, delete_build_raft_artifacts_folder, idf_path);
        if build_result.is_err() {
            return Err(Box::new(std::io::Error::new(std::io::ErrorKind::Other, "Build failed")));
        }
    } else {
        // Build with docker
        let build_result = build_with_docker(&app_folder, &sys_type, 
                    clean, delete_build_folder, delete_build_raft_artifacts_folder);
        if build_result.is_err() {
            return Err(Box::new(std::io::Error::new(std::io::ErrorKind::Other, "Build failed")));
        }
    }

    // Store a file in the "build_raft_artifacts" folder to indicate the SysType
    // of the last build
    let build_raft_artifacts_folder = format!("{}/build_raft_artifacts", app_folder);
    fs::create_dir_all(&build_raft_artifacts_folder)?;
    fs::write(format!("{}/cursystype.txt", build_raft_artifacts_folder), sys_type)?;

    Ok(())
}

// Check the app folder is valid
fn check_app_folder_valid(app_folder: &str, sys_types_base_folder_rel: &str) {
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

// Build with docker
fn build_with_docker(project_dir: &str, systype_name: &str, clean: bool,
            delete_build_folder: bool, delete_raft_artifacts_folder: bool) -> Result<(), std::io::Error> {
    // Build with docker
    println!("Building with docker in {} for SysType {} clean {}", project_dir, systype_name, clean);

    // Build the Docker image
    let fail_docker_image_msg = format!("Docker build command failed");
    let docker_image_build_args = vec!["build", "-t", "raftbuilder", "."];
    let docker_image_build_status = Command::new("docker")
        .current_dir(project_dir)
        .args(docker_image_build_args)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())        
        .status()
        .expect(&fail_docker_image_msg);

    if !docker_image_build_status.success() {
        eprintln!("Docker image build command failed");
        return Err(std::io::Error::new(std::io::ErrorKind::Other, "Docker image build command failed"));
    }
    else {
        println!("Docker image build command succeeded");
    }

    // Execute the Docker command to build the app
    let build_dir = format!("./build/{}", systype_name);
    let absolute_project_dir = fs::canonicalize(project_dir)?;
    let docker_compatible_project_dir = convert_path_for_docker(absolute_project_dir);
    let project_dir_full = format!("{}:/project", docker_compatible_project_dir?);

    // Command sequence
    let mut command_sequence = String::new();

    if delete_build_folder {
        command_sequence += &build_dir;
    }
    if delete_raft_artifacts_folder {
        command_sequence += "rm -rf ./build_raft_artifacts; ";
    }

    command_sequence += "idf.py -B ";
    command_sequence += &build_dir;
    if clean {
        command_sequence += " fullclean";
    }
    command_sequence += " build";

    let docker_run_args = vec![
        "run", "--rm",
        "-v", &project_dir_full,
        "-w", "/project",
        "raftbuilder",
        "/bin/bash", "-c", &command_sequence,
    ];

    // Print args
    println!("Docker run args: {:?}", docker_run_args);

    // Execute the Docker command and capture its output
    let docker_command = "docker";
    match execute_and_capture_output(docker_command, &docker_run_args, project_dir) {
        Ok(output) => {
            // The output contains the command to flash the app which will look something like:
            // /opt/esp/python_env/idf5.1_py3.8_env/bin/python ../opt/esp/idf/components/esptool_py/esptool/esptool.py -p (PORT) -b 460800 --before default_reset --after hard_reset --chip esp32  write_flash --flash_mode dio --flash_size 4MB --flash_freq 40m 0x1000 build/SysTypeMain/bootloader/bootloader.bin 0x8000 build/SysTypeMain/partition_table/partition-table.bin 0x1e000 build/SysTypeMain/ota_data_initial.bin 0x20000 build/SysTypeMain/SysTypeMain.bin 0x380000 build/SysTypeMain/fs.bin
            // Extract the command to flash the app using the esptool.py as the keyword to locate the start of the command
            let flash_command_start = output.find("esptool.py -p").unwrap();
            let flash_command = &output[flash_command_start..];
            println!("Flash command: {}", flash_command);
        },
        Err(e) => {
            eprintln!("Docker run failed: {}", e);
            return Err(e);
        }
    }

    Ok(())
}

// Build without docker
fn build_without_docker(project_dir: &str, systype_name: &str, clean: bool,
    delete_build_folder: bool, delete_raft_artifacts_folder: bool,
    idf_path: String) -> Result<(), std::io::Error> {
    
    // Build without docker
    println!("Building without docker in {} for SysType {} clean {}", project_dir, systype_name, clean);
    
    // Folders
    let build_dir = format!("build/{}", systype_name);
    let build_raft_artifacts_folder = format!("{}/build_raft_artifacts", project_dir);

    // Delete the build folder if required
    if delete_build_folder {
        let build_dir_full = format!("{}/{}", project_dir, build_dir);
        if Path::new(&build_dir_full).exists() {
            fs::remove_dir_all(&build_dir_full)?;
        }
    }

    // Delete the "build_raft_artifacts" folder if required
    if delete_raft_artifacts_folder {
        if Path::new(&build_raft_artifacts_folder).exists() {
            fs::remove_dir_all(&build_raft_artifacts_folder)?;
        }
    }

    // Command sequence
    let mut command_sequence = String::new();
    command_sequence += "-B ";
    command_sequence += &build_dir;
    if clean {
        command_sequence += " fullclean";
    }
    command_sequence += " build";

    // Args
    let idf_run_args: Vec<&str> = vec![
        &command_sequence,
    ];

    // Execute
    let idf_command = idf_path.as_str();

    println!("idf.py command: {} run args: {:?}", idf_command, idf_run_args);
    
    match execute_and_capture_output(idf_command, &idf_run_args, project_dir) {
        Ok(output) => {
            // The output contains the command to flash the app which will look something like:
            // /opt/esp/python_env/idf5.1_py3.8_env/bin/python ../opt/esp/idf/components/esptool_py/esptool/esptool.py -p (PORT) -b 460800 --before default_reset --after hard_reset --chip esp32  write_flash --flash_mode dio --flash_size 4MB --flash_freq 40m 0x1000 build/SysTypeMain/bootloader/bootloader.bin 0x8000 build/SysTypeMain/partition_table/partition-table.bin 0x1e000 build/SysTypeMain/ota_data_initial.bin 0x20000 build/SysTypeMain/SysTypeMain.bin 0x380000 build/SysTypeMain/fs.bin
            // Extract the command to flash the app using the esptool.py as the keyword to locate the start of the command
            let flash_command_start = output.find("esptool.py -p").unwrap();
            let flash_command = &output[flash_command_start..];
            println!("Flash command: {}", flash_command);
        },
        Err(e) => {
            eprintln!("idf.py build failed {}", e);
            return Err(e);
        }
    }

    Ok(())
}

fn convert_path_for_docker(path: PathBuf) -> Result<String, std::io::Error> {
    let path_str = path.into_os_string().into_string().unwrap();

    // Remove the '\\?\' prefix if present (Windows extended-length path)
    let trimmed_path = if path_str.starts_with("\\\\?\\") {
        &path_str[4..]
    } else {
        &path_str
    };

    // Replace backslashes with forward slashes
    let docker_path = trimmed_path.replace("\\", "/");

    Ok(docker_path)
}

fn execute_and_capture_output(command: &str, args: &[&str], cur_dir: &str) -> io::Result<String> {
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