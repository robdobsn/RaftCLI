
use std::collections::HashMap;
use std::process::{Command, Stdio};
use std::fs;
use std::io;
use std::path::Path;
use nix::unistd::{getuid, getgid};
use crate::raft_cli_utils::{default_esp_idf_version, find_matching_esp_idf, is_docker_available, is_esp_idf_env, prepare_esp_idf, utils_get_sys_type, write_build_info, read_build_info};
use crate::raft_cli_utils::check_app_folder_valid;
use crate::raft_cli_utils::execute_and_capture_output;
use crate::raft_cli_utils::convert_path_for_docker;
use crate::raft_cli_utils::CommandError;
use crate::raft_cli_utils::get_esp_idf_version_from_dockerfile;
use crate::raft_cli_utils::idf_version_ok;

pub fn build_raft_app(build_sys_type: &Option<String>, clean: bool, clean_only: bool, app_folder: String,
            force_docker_arg: bool, no_docker_arg: bool, 
            use_local_idf_matching_dockerfile_idf: bool, 
            idf_path_full: Option<String>) 
                            -> Result<String, Box<dyn std::error::Error>> {

    // println!("Building the app in folder: {} clean {} clean_only {} no_docker_arg {}", app_folder, clean, clean_only, no_docker_arg);

    // Check the app folder is valid
    if !check_app_folder_valid(app_folder.clone()) {
        return Err(Box::new(std::io::Error::new(std::io::ErrorKind::Other, "Invalid app folder")));
    }

    // Determine the Systype to build
    let sys_type = utils_get_sys_type(build_sys_type, app_folder.clone());
    if sys_type.is_err() {
        return Err(Box::new(std::io::Error::new(std::io::ErrorKind::Other, "Error determining SysType")));
    }
    let sys_type = sys_type.unwrap();

    // Flags indicating the build folder should be deleted
    let delete_build_folder = clean || clean_only;

    // Read previous build information
    let build_info = read_build_info(&app_folder);

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

    // Apply saved build method preference if no explicit flags set
    if !no_docker_arg && !force_docker_arg && !use_local_idf_matching_dockerfile_idf && idf_path_full.is_none() {
        if let Some(ref last_method) = build_info.last_build_method {
            if last_method == "docker" {
                if is_docker_available() {
                    force_docker = true;
                } else {
                    println!("Warning: Previous build used Docker but Docker is not available, falling back to local IDF");
                }
            } else if last_method == "local_idf" {
                no_docker = true;
            }
        }
    }

    // Handle building with or without docker
    let (build_result, actual_build_method, actual_idf_path, idf_path_was_explicit) = 
        if use_local_idf_matching_dockerfile_idf || no_docker || !is_docker_available() && !force_docker {
        // Get idf path - priority order:
        // 1. Explicit path from -e flag
        // 2. Saved explicit path from previous build
        // 3. IDF_PATH environment variable
        let mut idf_path = idf_path_full.clone();
        let mut path_was_explicit = idf_path_full.is_some();
        
        // If no explicit path and we have a saved explicit path, try to use it
        if idf_path.is_none() && build_info.last_idf_path_explicit {
            if let Some(ref saved_path) = build_info.last_idf_path {
                if Path::new(saved_path).exists() {
                    println!("Using saved ESP-IDF path: {}", saved_path);
                    idf_path = Some(saved_path.clone());
                    path_was_explicit = true;
                } else {
                    println!("Warning: Saved ESP-IDF path no longer exists: {}", saved_path);
                }
            }
        }
        
        // Fall back to environment variable if still none
        if idf_path.is_none() {
            idf_path = std::env::var("IDF_PATH").ok();
        }
        
        let actual_path = idf_path.clone();

        // Build without docker
        let result = build_without_docker(app_folder.clone(), sys_type.clone(), clean, clean_only,
                    delete_build_folder, idf_path);
        (result, "local_idf", actual_path, path_was_explicit)
    } else if is_docker_available() {
        // Build with docker
        let result = build_with_docker(app_folder.clone(), sys_type.clone(), clean, clean_only,
                    delete_build_folder);
        (result, "docker", None, false)
    } else 
    {
        // Either ESP IDF or docker must be available to build
        let result = Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            "Either ESP IDF or Docker must be available to build",
        ));
        (result, "unknown", None, false)
    };

    // If the build failed, return the error
    if build_result.is_err() {
        return Err(Box::new(build_result.unwrap_err()));
    }

    // Save complete build info to raft.info file after successful build
    if let Err(e) = write_build_info(
        &app_folder,
        &sys_type,
        actual_build_method,
        idf_path_was_explicit,
        actual_idf_path,
    ) {
        println!("Warning: Failed to write raft.info file: {}", e);
    }

    Ok(build_result.unwrap().to_string())
}

// Build with docker and return output as a string
fn build_with_docker(project_dir: String, systype_name: String, clean: bool, clean_only: bool,
            delete_build_folder: bool) -> Result<String, std::io::Error> {

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

    command_sequence += "idf.py -B ";
    command_sequence += &build_dir;
    if clean {
        command_sequence += " fullclean";
    }
    if !clean_only {
        command_sequence += " build";
    }

    // Get current user and group IDs to run Docker with same permissions as host
    let user_group = format!("{}:{}", getuid(), getgid());

    let docker_run_args = vec![
        "run", "--rm",
        "--user", &user_group,
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
    match execute_and_capture_output(docker_command.clone(), &docker_run_args, project_dir.clone(), HashMap::new()) {
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
fn build_without_docker(project_dir: String, systype_name: String, clean: bool, clean_only: bool,
    delete_build_folder: bool, idf_path: Option<String>) -> Result<String, std::io::Error> {
    
    // Debug
    println!(
        "Raft build SysType {} in {}{} (no Docker)",
        systype_name,
        project_dir,
        if clean { " (clean first)" } else { "" }
    );
    
    // Folders
    let build_dir = format!("build/{}", systype_name);

    // Delete build folder if required
    if delete_build_folder {
        let build_dir_full = format!("{}/{}", project_dir.clone(), build_dir);
        if Path::new(&build_dir_full).exists() {
            fs::remove_dir_all(&build_dir_full)?;
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
    
    // Get required ESP IDF version from Dockerfile
    let required_esp_idf_version = get_esp_idf_version_from_dockerfile(&project_dir).unwrap_or(default_esp_idf_version());

    // // TODO remove
    // println!("Required ESP-IDF version: {:?} esp_idf_env_set {:?}", esp_idf_version, is_esp_idf_env());

    // Check if we an ESP IDF environment is set and the version is correct
    let mut idf_env_vars_to_add: HashMap<String, String> = HashMap::new();
    let esp_idf_ok = is_esp_idf_env() && idf_version_ok(required_esp_idf_version.clone());
    if !esp_idf_ok {

        // Use the IDF path provided or the IDF_PATH environment variable
        let idf_path: Option<String> = idf_path.or_else(|| std::env::var("IDF_PATH").ok());

        // No ESP IDF found so try to find one
        let idf_found_at_path = find_matching_esp_idf(required_esp_idf_version.clone(), idf_path);

        // TODO remove
        println!("IDF found {:?}", idf_found_at_path);

        // Prepare the ESP-IDF environment
        if idf_found_at_path.is_some() {
            let idf_prep_result = prepare_esp_idf(idf_found_at_path.unwrap().as_path());
            if idf_prep_result.is_err() {
                return Err(std::io::Error::new(std::io::ErrorKind::Other, "No ESP-IDF environment variables found"));
            }
            idf_env_vars_to_add = idf_prep_result.unwrap();
        } else {
            return Err(std::io::Error::new(std::io::ErrorKind::Other, "No matching ESP-IDF found"));
        }
           
        // return Err(std::io::Error::new(std::io::ErrorKind::Other, "ESP-IDF environment not found"));
    }

    // Execute the command and handle the output
    let idf_py_command = "idf.py".to_string();
    match execute_and_capture_output(idf_py_command.clone(), &idf_run_args, project_dir.clone(), idf_env_vars_to_add) {
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

