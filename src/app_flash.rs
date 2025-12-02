use std::collections::HashMap;
use crate::app_ports::select_most_likely_port;
use crate::app_ports::PortsCmd;
use crate::raft_cli_utils::build_flash_command_args;
use crate::raft_cli_utils::get_flash_tool_cmd;
use crate::raft_cli_utils::execute_and_capture_output;
use crate::raft_cli_utils::get_build_folder_name;
use crate::raft_cli_utils::utils_get_sys_type;
use crate::raft_cli_utils::is_wsl;

pub fn flash_raft_app(
    build_sys_type: &Option<String>,
    app_folder: String,
    serial_port: Option<String>,
    native_serial_port: bool,
    vid: Option<String>,
    flash_baud: u32,
    flash_tool_opt: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {

    let sys_type = utils_get_sys_type(build_sys_type, app_folder.clone());
    if sys_type.is_err() {
        return Err(Box::new(std::io::Error::new(std::io::ErrorKind::Other, "Error determining SysType")));
    }
    let sys_type = sys_type.unwrap();

    // In WSL without native serial port flag, delegate to Windows raft.exe for flashing
    // This ensures proper USB serial port access and uses Windows-native esptool
    if is_wsl() && !native_serial_port {
        println!("WSL detected: Delegating flash operation to Windows raft.exe for USB serial port access");
        return flash_via_windows_raft(
            &sys_type,
            app_folder,
            serial_port,
            vid,
            flash_baud,
            flash_tool_opt,
        );
    }

    // Get build folder
    let build_folder = get_build_folder_name(sys_type.clone(), app_folder.clone());

    // Get flash tool
    let flash_cmd: String = get_flash_tool_cmd(flash_tool_opt, native_serial_port);

    // Extract port and baud rate arguments
    let port = if let Some(port) = serial_port {
        port
    } else {
        // Use select_most_likely_port if no specific port is provided
        let port_cmd = PortsCmd::new_with_vid(vid);
        match select_most_likely_port(&port_cmd, native_serial_port) {
            Some(p) => p.port_name,
            None => {
                println!("Error: No suitable port found");
                std::process::exit(1);
            }
        }
    };

    // Extract the arguments for the flash command
    let flash_cmd_args = build_flash_command_args(build_folder.clone(), &port, flash_baud);

    // Check for errors in the flash command and arguments
    if flash_cmd_args.is_err() {
        return Err(Box::new(std::io::Error::new(
            std::io::ErrorKind::Other,
            "Error extracting flash command arguments",
        )));
    }
    let flash_cmd_args = flash_cmd_args.unwrap();

    // Debug
    println!("Flash command: {}", flash_cmd.clone());
    println!("Flash command args: {:?}", flash_cmd_args);
    println!("Flash command app folder: {}", app_folder.clone());
    // println!("Flash command build folder: {}", build_folder);

    // Execute the flash command and check for errors
    let (output, success_flag) = execute_and_capture_output(flash_cmd.clone(), &flash_cmd_args, app_folder.clone(), HashMap::new())?;
    if !success_flag {
        // Check if the error is related to esptool module not found
        if output.contains("ModuleNotFoundError: No module named 'esptool'") {
            let err_msg = format!(
                "Flash failed: esptool module not found.\n\n\
                This error typically occurs when:\n\
                1. esptool is not properly installed in the current environment\n\
                2. You're in WSL and should let raftcli use Windows for flashing (don't use -n flag)\n\n\
                Solutions:\n\
                - If in WSL: Run without the -n (native-serial-port) flag to use Windows raft.exe\n\
                - Otherwise: Install esptool using: pip install esptool\n\n\
                Original error:\n{}", output
            );
            return Err(Box::new(std::io::Error::new(std::io::ErrorKind::Other, err_msg)));
        }
        let err_msg = format!("Flash executed with errors: {}", output);
        return Err(Box::new(std::io::Error::new(std::io::ErrorKind::Other, err_msg)));
    }

    Ok(())
}

// Delegate flashing to Windows raft.exe when in WSL
fn flash_via_windows_raft(
    sys_type: &str,
    app_folder: String,
    serial_port: Option<String>,
    vid: Option<String>,
    flash_baud: u32,
    flash_tool_opt: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut args = vec!["flash".to_string()];
    
    // Add system type
    args.push("-s".to_string());
    args.push(sys_type.to_string());
    
    // Add serial port if specified
    if let Some(port) = serial_port {
        args.push("-p".to_string());
        args.push(port);
    }
    
    // Add vendor ID if specified
    if let Some(v) = vid {
        args.push("-v".to_string());
        args.push(v);
    }
    
    // Add flash baud rate
    args.push("-f".to_string());
    args.push(flash_baud.to_string());
    
    // Add flash tool if specified
    if let Some(tool) = flash_tool_opt {
        args.push("-t".to_string());
        args.push(tool);
    }
    
    // Add native serial port flag to tell Windows raft.exe to use Windows serial ports
    args.push("-n".to_string());
    
    println!("Executing Windows raft.exe with args: {:?}", args);
    
    // Execute raft.exe and stream output
    let output = std::process::Command::new("raft.exe")
        .args(&args)
        .current_dir(&app_folder)
        .output();
    
    match output {
        Ok(result) => {
            // Print stdout
            print!("{}", String::from_utf8_lossy(&result.stdout));
            
            // Print stderr if any
            let stderr = String::from_utf8_lossy(&result.stderr);
            if !stderr.is_empty() {
                eprint!("{}", stderr);
            }
            
            if result.status.success() {
                Ok(())
            } else {
                Err(Box::new(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("Windows raft.exe flash command failed with exit code: {:?}", result.status.code()),
                )))
            }
        }
        Err(e) => {
            if e.kind() == std::io::ErrorKind::NotFound {
                Err(Box::new(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "Could not find raft.exe (Windows version of raftcli).\n\n\
                    When using WSL, raftcli needs the Windows version (raft.exe) to access USB serial ports.\n\n\
                    Please ensure:\n\
                    1. raftcli is installed on Windows: cargo install raftcli\n\
                    2. raft.exe is in your Windows PATH\n\
                    3. You can access Windows executables from WSL (try: raft.exe --version)\n\n\
                    Alternative: Use the -n flag to attempt flashing with native Linux tools (requires USBIPD or similar)",
                )))
            } else {
                Err(Box::new(e))
            }
        }
    }
}