use crate::raft_cli_utils::extract_flash_cmd_args;
use crate::raft_cli_utils::get_flash_tool_cmd;
use crate::raft_cli_utils::execute_and_capture_output;

pub fn flash_raft_app(app_folder: String, port: String,
                force_native_serial_port: bool, flash_baud: u32, 
                flash_tool_opt: Option<String>, build_cmd_output: String)
                    -> Result<(), Box<dyn std::error::Error>> {

    // Get flash tool
    let flash_cmd = get_flash_tool_cmd(flash_tool_opt, force_native_serial_port);

    // Extract the arguments for the flash command
    let flash_cmd_args = extract_flash_cmd_args(build_cmd_output, &port, flash_baud);

    // Check if the flash args are valid
    if flash_cmd_args.is_err() {
        return Err(flash_cmd_args.err().unwrap());
    }
    let flash_cmd_args = flash_cmd_args.unwrap();

    // Debug
    println!("Flash command: {}", flash_cmd);
    println!("Flash command args: {:?}", flash_cmd_args);
    println!("Flash command app folder: {}", app_folder);

    // Execute the flash command
    let result = execute_and_capture_output(&flash_cmd, &flash_cmd_args, &app_folder);
   
    // Check for flash error
    if result.is_err() {
        return Err(Box::new(std::io::Error::new(std::io::ErrorKind::Other, format!("Flash failed {:?}", result))));
    }

    Ok(())
}

// Alternate implementation using espflash tool?
// pub fn flash_raft_app(build_sys_type: &Option<String>, app_folder: String, port: String, flash_baud: u32,
//                 flash_tool_opt: Option<String>, build_cmd_output: String)