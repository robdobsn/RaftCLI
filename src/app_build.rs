// buid.rs - RaftCLI: Build (with or without Docker) the Raft app
// Rob Dobson 2024

use std::process::{Command, Stdio};
use std::fs;
use std::path::Path;
use std::io;
use crate::raft_cli_utils::{is_docker_available, is_esp_idf_env, utils_setup_systype_build_and_get_args, utils_get_sys_type_list};
use crate::raft_cli_utils::check_app_folder_valid;
use crate::raft_cli_utils::check_for_raft_artifacts_deletion;
use crate::raft_cli_utils::execute_and_capture_output;
use crate::raft_cli_utils::convert_path_for_docker;
use crate::raft_cli_utils::CommandError;
use crate::systype_config::{systype_config_extract_systype_info, SysTypeConfig};

pub fn build_raft_app(build_sys_type: &Option<String>, clean: bool, clean_only: bool, app_folder: String,
            force_docker_arg: bool, no_docker_arg: bool, idf_path_full: Option<String>) 
                            -> Result<String, Box<dyn std::error::Error>> {

    // println!("Building the app in folder: {} clean {} clean_only {} no_docker_arg {}", app_folder, clean, clean_only, no_docker_arg);

    // Check the app folder is valid
    if !check_app_folder_valid(app_folder.clone()) {
        return Err(Box::new(std::io::Error::new(std::io::ErrorKind::Other, "Invalid app folder")));
    }

    // Get list of systypes
    let sys_type_list = utils_get_sys_type_list(build_sys_type, app_folder.clone());
    if sys_type_list.is_err() {
        return Err(Box::new(std::io::Error::new(std::io::ErrorKind::Other, "Error determining SysType")));
    }
    if sys_type_list.as_ref().unwrap().is_empty() {
        return Err(Box::new(std::io::Error::new(std::io::ErrorKind::Other, "No SysType found")));
    }

    // Iterate through the SysTypes and build each one
    let mut build_str = String::new();
    for sys_type in sys_type_list.unwrap() {
        let build_result: Result<String, Box<dyn std::error::Error>> = build_raft_app_for_sys_type(sys_type.clone(), clean, clean_only, app_folder.clone(),
                    force_docker_arg, no_docker_arg, idf_path_full.clone());
        if build_result.is_err() {
            return build_result;
        }
        build_str += build_result.unwrap().as_str();
    }

    Ok(build_str)
}

// Build the app for a specific SysType
fn build_raft_app_for_sys_type(sys_type: String, clean: bool, clean_only: bool, app_folder: String,
            force_docker_arg: bool, no_docker_arg: bool, idf_path_full: Option<String>) 
                            -> Result<String, Box<dyn std::error::Error>> {

    // Get the configuration details for the systype
    let systype_config = systype_config_extract_systype_info(app_folder.clone(), sys_type.clone());

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
        if check_for_raft_artifacts_deletion(app_folder.clone(), sys_type.clone()) {
            delete_build_raft_artifacts_folder = true;
        }
    }

    // Determine if docker is to be used for build
    let mut no_docker = std::env::var("RAFT_NO_DOCKER").unwrap_or("false".to_string()) == "true";
    if no_docker_arg {
        no_docker = true;
    }

    // Determine if docker is to be forced for build
    let mut force_docker = std::env::var("RAFT_FORCE_DOCKER").unwrap_or("false".to_string()) == "true";
    if force_docker_arg {
        force_docker = true;
    }

    // Handle building with or without docker
    let build_result = if (is_esp_idf_env() && !force_docker) || no_docker {
        // Get idf path
        let idf_path = idf_path_full.unwrap_or("idf.py".to_string());

        // Build without docker
        build_without_docker(app_folder.clone(), sys_type.clone(), systype_config, clean, clean_only,
                    delete_build_folder, delete_build_raft_artifacts_folder, idf_path)
    } else if is_docker_available() {
        // Build with docker
        build_with_docker(app_folder.clone(), sys_type.clone(), systype_config, clean, clean_only,
                    delete_build_folder, delete_build_raft_artifacts_folder)
    } else 
    {
        // Either ESP IDF or docker must be available to build
        Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            "Either ESP IDF or Docker must be available to build",
        ))
    };

    // If the build failed, return the error
    if build_result.is_err() {
        return Err(Box::new(build_result.unwrap_err()));
    }

    Ok(build_result.unwrap().to_string())
}

// Build with docker and return output as a string
fn build_with_docker(project_dir: String, systype_name: String, systype_config: SysTypeConfig, clean: bool, clean_only: bool,
            delete_build_folder: bool, delete_raft_artifacts_folder: bool) -> Result<String, std::io::Error> {

    // Build with docker
    println!("Raft build SysType {} in {}{}",  systype_name, project_dir.clone(),
                    if clean { " (clean first)" } else { "" });

    // Build the Docker image
    let fail_docker_image_msg = format!("Docker build command failed");
    let docker_image_build_args = vec!["build", "-t", "raftbuilder", "."];
    let docker_image_build_status = Command::new("docker")
        .current_dir(project_dir.clone())
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
    let absolute_project_dir = fs::canonicalize(project_dir.clone())?;
    let docker_compatible_project_dir = convert_path_for_docker(absolute_project_dir);
    let project_dir_full = format!("{}:/project", docker_compatible_project_dir?);

    // Command sequence
    let mut command_sequence = String::new();

    if delete_build_folder {
        command_sequence += format!("rm -rf ./{}; ", build_dir).as_str();
    }
    if delete_raft_artifacts_folder {
        command_sequence += "rm -rf ./build_raft_artifacts; ";
    }

    // Get the build command sequence as a string vector
    let build_args = utils_setup_systype_build_and_get_args(project_dir.clone(), build_dir.clone(), systype_name.clone(), systype_config, clean, clean_only);

    // Append the build command to the command sequence
    command_sequence += "idf.py";
    command_sequence += build_args.join(" ").as_str();

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
    // println!("Docker run args: {:?}", docker_run_args);

    // Execute the Docker command and capture its output
    let docker_command = "docker".to_string();
    match execute_and_capture_output(docker_command.clone(), &docker_run_args, project_dir.clone()) {
        Ok((output, success_flag)) => {
            if success_flag {
                // Success - return the output as a String
                Ok(output)
            } else {
                // If the command executed but was not successful, log the output and return an error
                eprintln!("Docker run failed but executed: {}", output);
                Err(io::Error::new(io::ErrorKind::Other, "Docker run executed with errors"))
            }
        },
        Err(e) => {
            // More granular error handling based on the CommandError enum
            let error_message = match e {
                CommandError::CommandNotFound(msg) => format!("Docker command not found: {}", msg),
                CommandError::ExecutionFailed(msg) => format!("Docker execution failed: {}", msg),
                CommandError::Other(io_err) => format!("An IO error occurred during Docker execution: {}", io_err),
            };
            eprintln!("Docker run failed: {}", error_message);
            Err(io::Error::new(io::ErrorKind::Other, error_message))
        }
    }
}

// Build without docker
fn build_without_docker(project_dir: String, systype_name: String, systype_config: SysTypeConfig, clean: bool, clean_only: bool,
    delete_build_folder: bool, delete_raft_artifacts_folder: bool,
    idf_path: String) -> Result<String, std::io::Error> {
    
    // Build with docker
    println!("Raft build SysType {} in {}{} (no Docker)",  systype_name, project_dir.clone(),
                    if clean { " (clean first)" } else { "" });
    
    // Folders
    let build_dir = format!("build/{}", systype_name);
    let build_raft_artifacts_folder = format!("{}/build_raft_artifacts", project_dir.clone());

    // Delete build folders if required
    if delete_build_folder {
        let build_dir_full = format!("{}/{}", project_dir.clone(), build_dir);
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
    let idf_build_args = utils_setup_systype_build_and_get_args(project_dir.clone(), build_dir.clone(), systype_name.clone(), systype_config, clean, clean_only);
    
    // Execute the command and handle the output
    match execute_and_capture_output(idf_path.clone(), &idf_build_args, project_dir.clone()) {
        Ok((output, success_flag)) => {
            if success_flag {
                Ok(output) // Return the output directly
            } else {
                // If the command executed but failed, provide detailed feedback
                eprintln!("idf.py build executed but failed: {}", output);
                Err(io::Error::new(io::ErrorKind::Other, "idf.py build executed with errors"))
            }
        },
        Err(e) => {
            // Detailed error handling based on the failure
            let error_message = match e {
                CommandError::CommandNotFound(msg) => {
                    // Check if the error is due to the idf.py command not being found
                    if msg.contains("idf.py") {
                        "idf.py command was not found - see https://docs.espressif.com/projects/esp-idf/en/stable/esp32/get-started/index.html".to_string()
                    } else {
                        format!("Command not found: {}", msg)
                    }
                },
                CommandError::ExecutionFailed(msg) => format!("Execution failed: {}", msg),
                CommandError::Other(io_err) => format!("An IO error occurred: {}", io_err),
            };
            eprintln!("idf.py build failed: {}", error_message);
            Err(io::Error::new(io::ErrorKind::Other, error_message))
        }
    }
}
