use crate::raft_cli_utils::utils_get_sys_type;
use std::fs;
use std::env;

pub fn flash_raft_app(build_sys_type: &Option<String>, app_folder: String, port: String, baud: u32, flash_tool_opt: Option<String>) 
                    -> Result<(), Box<dyn std::error::Error>> {

    // Determine the Systype to build
    let sys_type = utils_get_sys_type(build_sys_type, &app_folder);

    // Get the app folder
    let app_folder_path = app_folder;

    // Get the app build folder
    let build_folder = format!("{}/build", app_folder_path);

    // Get the app bin folder
    let bin_folder = format!("{}/{}", build_folder, sys_type);

    // Get the app bin file
    let bin_file_path = format!("{}/{}.elf", bin_folder, sys_type);

    // // Tool for programming the chip
    // let flash_tool: String;

    // // If the tool is specified then use it
    // match flash_tool_opt 

    // Check if running under WSL by looking for WSL-specific environment variable or file content
    let is_wsl = env::var("WSL_DISTRO_NAME").is_ok() || fs::read_to_string("/proc/version")
    .map(|contents| contents.contains("Microsoft") || contents.contains("WSL"))
    .unwrap_or(false);

    let esptool_command = if is_wsl {
        // If under WSL, use the Windows version of esptool
        "esptool.exe"
    } else {
        // Otherwise, use the Linux version
        "esptool"
    };

    // Flash tool to use
    let flash_tool = flash_tool_opt.unwrap_or("esptool".to_string());

    // Check the app folder is valid
    println!("Flashing app sys_type {} app_folder {} port {} baud {} bin_file_path {:?} flash_tool {} cmd {} is_wsl {}",
                &sys_type, app_folder_path, port, baud, bin_file_path, flash_tool, esptool_command, is_wsl);

    


    Ok(())
}
