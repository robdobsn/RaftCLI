
use std::process::{Command, Stdio};
use std::fs;
use std::path::Path;
use crate::raft_cli_utils::utils_get_sys_type;
use crate::raft_cli_utils::check_app_folder_valid;
use crate::raft_cli_utils::check_for_raft_artifacts_deletion;
use crate::raft_cli_utils::execute_and_capture_output;
use crate::raft_cli_utils::convert_path_for_docker;

pub fn build_raft_app(build_sys_type: &Option<String>, clean: bool, app_folder: String,
            no_docker_arg: bool, idf_path_full: Option<String>) 
                            -> Result<(), Box<dyn std::error::Error>> {

    // Check the app folder is valid
    check_app_folder_valid(&app_folder);

    // Determine the Systype to build
    let sys_type = utils_get_sys_type(build_sys_type, &app_folder);

    // Flags indicating the build folder and "build_raft_artifacts" folder should be deleted
    let mut delete_build_folder = false;
    let mut delete_build_raft_artifacts_folder = false;

    // If clean is true, delete the build folder for the SysType to built and
    // the "build_raft_artifacts" folder
    if clean {
        delete_build_folder = true;
        delete_build_raft_artifacts_folder = true;
    } else {
        // Check if the "build_raft_artifacts" folder needs to be deleted
        if check_for_raft_artifacts_deletion(&app_folder, &sys_type) {
            delete_build_raft_artifacts_folder = true;
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

    // IDF args in a vector
    let mut idf_run_args = vec!["-B", &build_dir];
    if clean {
        idf_run_args.push("fullclean");
    }
    idf_run_args.push("build");

    // Command
    let idf_command = idf_path.as_str();
    
    // Debug
    println!("Build app command: {} {:?}", idf_command, idf_run_args);
    
    // Execute the command and capture its output
    match execute_and_capture_output(idf_command, &idf_run_args, project_dir) {
        Ok(output) => {
            // The output contains the command to flash the app which will look something like:
            // /opt/esp/python_env/idf5.1_py3.8_env/bin/python ../opt/esp/idf/components/esptool_py/esptool/esptool.py -p (PORT) -b 460800 --before default_reset --after hard_reset --chip esp32  write_flash --flash_mode dio --flash_size 4MB --flash_freq 40m 0x1000 build/SysTypeMain/bootloader/bootloader.bin 0x8000 build/SysTypeMain/partition_table/partition-table.bin 0x1e000 build/SysTypeMain/ota_data_initial.bin 0x20000 build/SysTypeMain/SysTypeMain.bin 0x380000 build/SysTypeMain/fs.bin
            // Extract the command to flash the app using the esptool.py as the keyword to locate the start of the command
            let flash_command_start = output.find("esptool.py -p");
            match flash_command_start {
                Some(start) => {
                    let flash_command = &output[start..];
                    println!("Flash command: {}", flash_command);
                },
                None => {
                    eprintln!("Flash command not found in output");
                }
            }
        },
        Err(e) => {
            eprintln!("idf.py build failed {}", e);
            return Err(e);
        }
    }

    Ok(())
}
