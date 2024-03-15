
use std::process::{Command, Stdio};
use std::fs;
use std::path::Path;
use crate::raft_cli_utils::utils_get_sys_type;
use crate::raft_cli_utils::check_app_folder_valid;
use crate::raft_cli_utils::check_for_raft_artifacts_deletion;
use crate::raft_cli_utils::execute_and_capture_output;
use crate::raft_cli_utils::convert_path_for_docker;

pub fn build_raft_app(build_sys_type: &Option<String>, clean: bool, clean_only: bool, app_folder: String,
            no_docker_arg: bool, idf_path_full: Option<String>) 
                            -> Result<String, Box<dyn std::error::Error>> {

    // println!("Building the app in folder: {} clean {} clean_only {} no_docker_arg {}", app_folder, clean, clean_only, no_docker_arg);

    // Check the app folder is valid
    if !check_app_folder_valid(&app_folder) {
        return Err(Box::new(std::io::Error::new(std::io::ErrorKind::Other, "Invalid app folder")));
    }

    // Determine the Systype to build
    let sys_type = utils_get_sys_type(build_sys_type, &app_folder);
    if sys_type.is_err() {
        return Err(Box::new(std::io::Error::new(std::io::ErrorKind::Other, "Error determining SysType")));
    }
    let sys_type = sys_type.unwrap();

    // Flags indicating the build folder and "build_raft_artifacts" folder should be deleted
    let mut delete_build_folder = false;
    let mut delete_build_raft_artifacts_folder = false;

    // If clean or clean_only is true, delete the build folder for the SysType to built and
    // the "build_raft_artifacts" folder
    if clean || clean_only {
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
    let build_result = if no_docker {
        // Get idf path
        let idf_path = idf_path_full.unwrap_or("idf.py".to_string());

        // Build without docker
        build_without_docker(&app_folder, &sys_type, clean, clean_only,
                    delete_build_folder, delete_build_raft_artifacts_folder, idf_path)
    } else {
        // Build with docker
        build_with_docker(&app_folder, &sys_type, clean, clean_only,
                    delete_build_folder, delete_build_raft_artifacts_folder)
    };

    // Check for build error
    if build_result.is_err() {
        return Err(Box::new(std::io::Error::new(std::io::ErrorKind::Other, "Build failed")));
    }

    // Store a file in the "build_raft_artifacts" folder to indicate the SysType
    // of the last build
    let build_raft_artifacts_folder = format!("{}/build_raft_artifacts", app_folder);
    fs::create_dir_all(&build_raft_artifacts_folder)?;
    fs::write(format!("{}/cursystype.txt", build_raft_artifacts_folder), sys_type)?;

    Ok(build_result.unwrap().to_string())
}

// Build with docker and return output as a string
fn build_with_docker(project_dir: &str, systype_name: &str, clean: bool, clean_only: bool,
            delete_build_folder: bool, delete_raft_artifacts_folder: bool) -> Result<String, std::io::Error> {
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
    if !clean_only {
        command_sequence += " build";
    }

    let docker_run_args = vec![
        "run", "--rm",
        "-v", &project_dir_full,
        "-w", "/project",
        "raftbuilder",
        "/bin/bash", "-c", &command_sequence,
    ];

    // Convert to string vector
    let docker_run_args: Vec<String> = docker_run_args.iter().map(|s| s.to_string()).collect();

    // Print args
    println!("Docker run args: {:?}", docker_run_args);

    // Execute the Docker command and capture its output
    let docker_command = "docker";
    match execute_and_capture_output(docker_command, &docker_run_args, project_dir) {
        Ok(output) => {
            return Ok(output);
        },
        Err(e) => {
            eprintln!("Docker run failed: {}", e);
            return Err(e);
        }
    }
}

// Build without docker
fn build_without_docker(project_dir: &str, systype_name: &str, clean: bool, clean_only: bool,
    delete_build_folder: bool, delete_raft_artifacts_folder: bool,
    idf_path: String) -> Result<String, std::io::Error> {
    
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

    // IDF args in a vector of Strings
    let mut idf_run_args = vec!["-B".to_string(), build_dir];
    if clean {
        idf_run_args.push("fullclean".to_string());
    }
    if !clean_only {
        idf_run_args.push("build".to_string());
    }

    // Command
    let idf_command = idf_path.as_str();
    
    // Debug
    println!("Build app command: {} {:?}", idf_command, idf_run_args);
    
    // Execute the command and capture its output
    match execute_and_capture_output(idf_command, &idf_run_args, project_dir) {
        Ok(output) => {
            return Ok(output);
        },
        Err(e) => {
            eprintln!("idf.py build failed {}", e);
            return Err(e);
        }
    }
}
