use std::collections::HashMap;
use crate::app_ports::select_most_likely_port;
use crate::app_ports::PortsCmd;
use crate::raft_cli_utils::build_flash_command_args;
use crate::raft_cli_utils::get_flash_tool_cmd;
use crate::raft_cli_utils::execute_and_capture_output;
use crate::raft_cli_utils::get_build_folder_name;
use crate::raft_cli_utils::utils_get_sys_type;

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
        let err_msg = format!("Flash executed with errors: {}", output);
        return Err(Box::new(std::io::Error::new(std::io::ErrorKind::Other, err_msg)));
    }

    Ok(())
}