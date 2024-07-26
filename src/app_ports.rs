use serialport_fix_stop_bits::{available_ports, SerialPortType, SerialPortInfo, UsbPortInfo};
use clap::Parser;
use wildmatch::WildMatch;
use std::error::Error;

use crate::raft_cli_utils::is_wsl;

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

const DEFAULT_PREFERRED_VIDS: &[&str] = &[
    "303a", // Espressif
    "2886", // Seeed
    "0403", // FTDI
    "10C4", // Silicon Labs
    "2341", // Arduino
    "239a", // Adafruit
];

pub fn manage_ports(cmd: &PortsCmd) {
    if let Err(e) = list_ports(cmd) {
        println!("Error listing ports: {}", e);
        std::process::exit(1);
    }
}

fn matches(str: &str, pattern: Option<String>, debug: bool) -> bool {
    let result = match pattern {
        Some(ref pattern) => {
            if pattern.contains('*') || pattern.contains('?') {
                WildMatch::new(pattern).matches(str)
            } else {
                WildMatch::new(&format!("*{}*", pattern)).matches(str)
            }
        }
        None => true,
    };
    if debug {
        println!("matches(str:{:?}, pattern:{:?}) -> {:?}", str, pattern, result);
    }
    result
}

fn matches_opt(str: Option<String>, pattern: Option<String>, debug: bool) -> bool {
    if let Some(str) = str {
        matches(&str, pattern, debug)
    } else {
        let result = pattern.is_none();
        if debug {
            println!("matches_opt(str:{:?}, pattern:{:?}) -> {:?}", str, pattern, result);
        }
        result
    }
}

fn usb_port_matches(port: &SerialPortInfo, cmd: &PortsCmd) -> bool {
    if let SerialPortType::UsbPort(info) = &port.port_type {
        if matches(&port.port_name, cmd.port.clone(), cmd.debug)
            && matches(&format!("{:04x}", info.vid), cmd.vid.clone(), cmd.debug)
            && matches(&format!("{:04x}", info.pid), cmd.pid.clone(), cmd.debug)
            && matches_opt(info.manufacturer.clone(), cmd.manufacturer.clone(), cmd.debug)
            && matches_opt(info.serial_number.clone(), cmd.serial.clone(), cmd.debug)
            && matches_opt(info.product.clone(), cmd.product.clone(), cmd.debug)
        {
            return true;
        }
    }
    false
}

fn sort_ports(mut ports: Vec<SerialPortInfo>, cmd: &PortsCmd) -> Vec<SerialPortInfo> {
    let preferred_vids: Vec<&str> = cmd.preferred_vids.as_ref()
        .map(|vids| vids.split(',').collect())
        .unwrap_or_else(|| DEFAULT_PREFERRED_VIDS.to_vec());

    ports.sort_by_key(|port| {
        if let SerialPortType::UsbPort(info) = &port.port_type {
            if preferred_vids.contains(&format!("{:04x}", info.vid).as_str()) {
                0
            } else {
                1
            }
        } else {
            1
        }
    });
    ports
}

fn filtered_ports(cmd: &PortsCmd) -> Result<Vec<SerialPortInfo>, Box<dyn Error>> {
    let mut ports: Vec<SerialPortInfo> = available_ports()?
        .into_iter()
        .filter(|info| usb_port_matches(info, cmd))
        .collect();
    ports.sort_by(|a, b| a.port_name.cmp(&b.port_name));
    ports = sort_ports(ports, cmd);
    if let Some(index) = cmd.index {
        if index < ports.len() {
            Ok(vec![ports[index].clone()])
        } else {
            Ok(Vec::new())
        }
    } else if ports.is_empty() {
        Ok(Vec::new())
    } else {
        Ok(ports)
    }
}

fn extra_usb_info(info: &UsbPortInfo) -> String {
    let mut output = String::new();
    output = output + &format!("{:04x}:{:04x}", info.vid, info.pid);
    let mut extra_items = Vec::new();

    if let Some(manufacturer) = &info.manufacturer {
        extra_items.push(format!("manufacturer '{}'", manufacturer));
    }
    if let Some(serial) = &info.serial_number {
        extra_items.push(format!("serial '{}'", serial));
    }
    if let Some(product) = &info.product {
        extra_items.push(format!("product '{}'", product));
    }
    if !extra_items.is_empty() {
        output += " ";
        output += &extra_items.join(" ");
    }
    output
}

fn list_ports(cmd: &PortsCmd) -> Result<(), Box<dyn Error>> {
    let ports_list = filtered_ports(cmd)?;
    if ports_list.is_empty() {
        println!("No ports found");
    } else {
        for port in ports_list {
            if let SerialPortType::UsbPort(info) = &port.port_type {
                println!(
                    "{} USB {}",
                    port.port_name,
                    extra_usb_info(&info)
                );
            } else {
                println!("{} Serial Device", port.port_name);
            }
        }
    }
    Ok(())
}

pub fn select_most_likely_port(cmd: &PortsCmd, native_serial_port: bool) -> Option<SerialPortInfo> {
    // println!("select_most_likely_port cmd: {:?} native_serial_port: {:?}", cmd, native_serial_port);
    if is_wsl() && !native_serial_port {
        // println!("WSL detected, looking for windows serial ports");
        
        // Use raft.exe ports <-v vid> to get the list of ports
        let mut args = vec!["ports"];
        if let Some(vid) = &cmd.vid {
            args.push("-v");
            args.push(vid);
        }
        let output = std::process::Command::new("raft.exe")
            .args(args)
            .output()
            .expect("Failed to execute raft.exe ports");
        let output = String::from_utf8_lossy(&output.stdout);
        // println!("select_most_likely_port output: {:?}", output);
        
        // Check for "No ports" message (no ports found)
        let no_ports_msg_pattern = "No ports";
        if output.contains(no_ports_msg_pattern) {
            // println!("No suitable serial ports found");
            return None;
        }
        let lines: Vec<&str> = output.lines().collect();
        let mut ports: Vec<SerialPortInfo> = Vec::new();
        for line in lines {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() > 1 {
                let port_name = parts[0].to_string();
                let port_type = SerialPortType::UsbPort(UsbPortInfo {
                    vid: 0x0403,
                    pid: 0x0000,
                    manufacturer: Some("FTDI".to_string()),
                    serial_number: None,
                    product: None,
                });
                ports.push(SerialPortInfo {
                    port_name,
                    port_type,
                });
            }
        }
        if !ports.is_empty() {
            // println!("select_most_likely_port found ports {:?}", ports);
            return Some(ports[0].clone());
        }
    }
    if let Ok(ports) = filtered_ports(cmd) {
        if !ports.is_empty() {
            // println!("select_most_likely_port found ports {:?}", ports);
            return Some(ports[0].clone());
        }
    }
    // println!("No ports found");
    None
}
