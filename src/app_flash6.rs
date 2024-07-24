use crate::raft_cli_utils::extract_flash_cmd_args;
use crate::raft_cli_utils::get_flash_tool_cmd;
use crate::raft_cli_utils::get_device_type;
use crate::raft_cli_utils::get_build_folder_name;
use crate::raft_cli_utils::utils_get_sys_type;
use espflash::flasher::{Flasher, FlashDataBuilder, FlashSettings};
use espflash::connection::Port;
use espflash::targets::Chip;
use serialport::available_ports;

pub fn flash_raft_app(build_sys_type: &Option<String>, 
    app_folder: String, port: String,
    native_serial_port: bool, 
    flash_baud: u32, 
    flash_tool_opt: Option<String>, 
    build_cmd_output: String)
    -> Result<(), Box<dyn std::error::Error>> {

    // Get flash tool
    let flash_cmd: String = get_flash_tool_cmd(flash_tool_opt, native_serial_port);

    // Get SysType
    let sys_type = utils_get_sys_type(build_sys_type, app_folder.clone());
    if sys_type.is_err() {
        return Err(Box::new(std::io::Error::new(std::io::ErrorKind::Other, "Error determining SysType")));
    }
    let sys_type: String = sys_type.unwrap();

    // Get device type string
    let device_type = get_device_type(sys_type.clone(), app_folder.clone());

    // Extract the arguments for the flash command
    let flash_cmd_args = extract_flash_cmd_args(build_cmd_output, device_type, &port, flash_baud);

    // Check for errors in the flash command and arguments
    if flash_cmd_args.is_err() {
        return Err(Box::new(std::io::Error::new(std::io::ErrorKind::Other, "Error extracting flash command arguments")));
    }
    let flash_cmd_args = flash_cmd_args.unwrap();

    // Get build folder
    let build_folder = get_build_folder_name(sys_type.clone(), app_folder.clone());

    // Debug
    println!("Flash command: {}", flash_cmd.clone());
    println!("Flash command args: {:?}", flash_cmd_args);
    println!("Flash command app folder: {}", app_folder.clone());
    println!("Flash command build folder: {}", build_folder);

    // // Execute the flash command and check for errors
    // let (output, success_flag) = execute_and_capture_output(flash_cmd.clone(), &flash_cmd_args, build_folder.clone())?;
    // if !success_flag {
    //     let err_msg = format!("Flash executed with errors: {}", output);
    //     return Err(Box::new(std::io::Error::new(std::io::ErrorKind::Other, err_msg)));
    // }

    // Get the USB port info
    let usb_info = available_ports()?
    .into_iter()
    .find(|p| p.port_name == port)
    .and_then(|p| match p.port_type {
        serialport::SerialPortType::UsbPort(info) => Some(info),
        _ => None,
    });

    // Open the serial port
    let serial_port = serialport::new(&port, 115_200)
        .timeout(std::time::Duration::from_secs(3))
        .open()?;

    let usb_info = usb_info.ok_or("No USB info found for the port")?;

    // Set up the flasher
    let mut flasher = Flasher::connect(
        Port::new(serial_port),
        usb_info,
        None,
        true,  // use_stub
        true,  // verify
        false, // skip
        Some(Chip::Esp32), // specify the chip type
        espflash::connection::reset::ResetAfterOperation::HardReset,
        espflash::connection::reset::ResetBeforeOperation::DefaultReset,
    )?;

    // Load the firmware image
    // let firmware_data = std::fs::read(build_folder)?;

    // // Create FlashData with default settings
    // let flash_data = FlashDataBuilder::new()
    //     .with_flash_settings(FlashSettings::default())
    //     .build()?;

    // // Flash the firmware to the device
    // flasher.write_bin_to_flash(0x1000, &firmware_data, None)?;

    Ok(())
}

// Alternate implementation using espflash tool?
// pub fn flash_raft_app(build_sys_type: &Option<String>, app_folder: String, port: String, flash_baud: u32,
//                 flash_tool_opt: Option<String>, build_cmd_output: String)