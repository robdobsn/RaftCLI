use serialport_fix_stop_bits::{available_ports, SerialPortType, SerialPortInfo, UsbPortInfo};
use clap::Parser;
use wildmatch::WildMatch;
use std::error::Error;
use std::path::Path;
use std::time::Duration;
use espflash::flasher::{Flasher, FlashDataBuilder, FlashSettings};
use espflash::targets::Chip;
use serialport::prelude::*;

#[derive(Clone, Parser, Debug)]
pub struct PortsCmd {
    #[clap(short = 'p', long, help = "Port pattern")]
    pub port: Option<String>,
    #[clap(short = 'v', long, help = "Vendor ID")]
    pub vid: Option<String>,
    #[clap(short = 'd', long, help = "Product ID")]
    pub pid: Option<String>,
    #[clap(long, help = "Manufacturer")]
    pub manufacturer: Option<String>,
    #[clap(long, help = "Serial number")]
    pub serial: Option<String>,
    #[clap(long, help = "Product name")]
    pub product: Option<String>,
    #[clap(short = 'i', long, help = "Index")]
    pub index: Option<usize>,
    #[clap(short = 'D', long, help = "Debug mode")]
    pub debug: bool,
    #[clap(long, help = "Preferred VIDs (comma separated list)")]
    pub preferred_vids: Option<String>,
}

impl PortsCmd {
    pub fn new_with_vid(vid: Option<String>) -> Self {
        PortsCmd {
            port: None,
            vid,
            pid: None,
            manufacturer: None,
            serial: None,
            product: None,
            index: None,
            debug: false,
            preferred_vids: None,
        }
    }
}

fn flash_firmware(port_name: &str, firmware_path: &str) -> Result<(), Box<dyn Error>> {
    // Open the serial port
    let serial_port = serialport_fix_stop_bits::new(port_name, 115_200)
        .timeout(Duration::from_secs(3))
        .open()?;

    // Get the USB port info (required by the flasher)
    #[cfg(unix)]
    let usb_info = if let Some(usb_port) = serial_port.as_ref().as_any().downcast_ref::<TTYPort>() {
        usb_port.port_info().usb_info
    } else {
        None
    };

    #[cfg(windows)]
    let usb_info = if let Some(usb_port) = serial_port.as_ref().as_any().downcast_ref::<COMPort>() {
        usb_port.port_info().usb_info
    } else {
        None
    };

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
        espflash::connection::reset::ResetAfterOperation::Reset,
        espflash::connection::reset::ResetBeforeOperation::Reset,
    )?;

    // Load the firmware image
    let firmware_data = std::fs::read(firmware_path)?;

    // Create FlashData with default settings
    let flash_data = FlashDataBuilder::new()
        .with_flash_settings(FlashSettings::default())
        .build()?;

    // Flash the firmware to the device
    flasher.write_bin_to_flash(0x1000, &firmware_data, None)?;

    Ok(())
}

fn main() {
    let port_name = "/dev/ttyUSB0"; // Replace with your serial port
    let firmware_path = "path/to/your/firmware.bin"; // Replace with the path to your firmware

    match flash_firmware(port_name, firmware_path) {
        Ok(_) => println!("Firmware flashed successfully!"),
        Err(e) => eprintln!("Failed to flash firmware: {:?}", e),
    }
}
