use crate::raft_cli_utils::{
    extract_flash_cmd_args, get_build_folder_name, get_device_type, get_flash_tool_cmd,
    utils_get_sys_type,
};
use espflash::flasher::{FlashDataBuilder, FlashSettings, Flasher};
use espflash::targets::Chip;
use serialport_fix_stop_bits::{new, available_ports, UsbPortInfo};
use std::error::Error;
use std::path::Path;
use std::time::Duration;

pub fn flash_raft_app(
    build_sys_type: &Option<String>,
    app_folder: String,
    port: String,
    native_serial_port: bool,
    flash_baud: u32,
    flash_tool_opt: Option<String>,
    build_cmd_output: String,
) -> Result<(), Box<dyn Error>> {
    // Get flash tool
    let flash_cmd: String = get_flash_tool_cmd(flash_tool_opt, native_serial_port);

    // Get SysType
    let sys_type = utils_get_sys_type(build_sys_type, app_folder.clone());
    if sys_type.is_err() {
        return Err(Box::new(std::io::Error::new(
            std::io::ErrorKind::Other,
            "Error determining SysType",
        )));
    }
    let sys_type: String = sys_type.unwrap();

    // Get device type string
    let device_type = get_device_type(sys_type.clone(), app_folder.clone());

    // Extract the arguments for the flash command
    let flash_cmd_args = extract_flash_cmd_args(build_cmd_output, device_type, &port, flash_baud);

    // Check for errors in the flash command and arguments
    if flash_cmd_args.is_err() {
        return Err(Box::new(std::io::Error::new(
            std::io::ErrorKind::Other,
            "Error extracting flash command arguments",
        )));
    }
    let flash_cmd_args = flash_cmd_args.unwrap();

    // Get build folder
    let build_folder = get_build_folder_name(sys_type.clone(), app_folder.clone());

    // Debug
    println!("Flash command: {}", flash_cmd.clone());
    println!("Flash command args: {:?}", flash_cmd_args);
    println!("Flash command app folder: {}", app_folder.clone());
    println!("Flash command build folder: {}", build_folder);

    // Open the serial port
    let serial_port = new(&port, 115_200)
        .timeout(Duration::from_secs(3))
        .open()?;

    // Get the USB port info
    let usb_info = available_ports()?
        .into_iter()
        .find(|p| p.port_name == port)
        .and_then(|p| match p.port_type {
            serialport_fix_stop_bits::SerialPortType::UsbPort(info) => Some(info),
            _ => None,
        })
        .ok_or("No USB info found for the port")?;

    // Set up the flasher
    let mut flasher = Flasher::connect(
        serial_port,
        usb_info,
        None,
        true,  // use_stub
        true,  // verify
        false, // skip
        Some(Chip::Esp32), // specify the chip type
        espflash::connection::reset::ResetAfterOperation::Reset,
        espflash::connection::reset::ResetBeforeOperation::Reset,
    )?;

    // Load the firmware image
    let firmware_path = Path::new(&app_folder).join("build").join("firmware.bin");
    let firmware_data = std::fs::read(&firmware_path)?;

    // Create FlashData with default settings
    let flash_data = FlashDataBuilder::new()
        .with_flash_settings(FlashSettings::default())
        .build()?;

    // Flash the firmware to the device
    flasher.write_bin_to_flash(0x1000, &firmware_data, None)?;

    Ok(())
}
