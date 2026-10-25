// RaftCLI: raft ble - Raft devices over BLE
// raft ble scan lists the Raft devices in range (those advertising a RICREST BLE service), to find the name to use
// in ble:<name>. Device commands (raft fs, raft devtypes) connect to BLE devices themselves.
// Rob Dobson 2026

use clap::{Parser, Subcommand};

use crate::device_conn::{run_windows_raft, BleArgs, CmdResult};
use crate::raft_cli_utils::is_wsl;

#[derive(Clone, Parser, Debug)]
pub struct BleCmd {
    #[clap(subcommand)]
    pub action: BleAction,
}

#[derive(Clone, Subcommand, Debug)]
pub enum BleAction {
    /// List the Raft devices in range: name, address, signal strength and service
    Scan {
        /// How long to scan for (seconds)
        #[clap(short = 't', long, default_value = "5")]
        timeout: u64,
        /// In WSL, use this system's Bluetooth (by default the Windows raft.exe scans)
        #[clap(short = 'n', long)]
        native: bool,
        #[clap(flatten)]
        ble: BleArgs,
    },
}

pub fn ble_app(cmd: BleCmd) -> CmdResult<()> {
    match cmd.action {
        BleAction::Scan { timeout, native, ble } => {
            // WSL has no Bluetooth: scan from Windows
            if !native && is_wsl() {
                let mut args: Vec<String> = std::env::args().skip(1).collect();
                args.push("-n".to_string());
                return run_windows_raft(&args, "the BLE scan");
            }
            scan(timeout, &ble)
        }
    }
}

#[cfg(feature = "ble")]
fn scan(timeout: u64, ble: &BleArgs) -> CmdResult<()> {
    use crate::link::ble::scan_devices;
    use std::time::Duration;

    eprintln!("Scanning for {} s...", timeout);
    let found = scan_devices(Duration::from_secs(timeout), &ble.options())?;
    if found.is_empty() {
        println!(
            "No Raft devices found. A device that is connected to something else doesn't advertise - Windows can \
             keep a BLE link after a program exits. Release it by typing bledisconnect in raft monitor, with \
             http://<device>/api/bledisconnect over WiFi, or by restarting it"
        );
        return Ok(());
    }
    let name_width = found.iter().map(|d| d.name.len()).max().unwrap_or(0).max(4);
    println!("{:<name_width$}  {:<17}  {:>4}  SERVICE", "NAME", "ADDRESS", "RSSI");
    for d in &found {
        let rssi = d.rssi.map(|r| r.to_string()).unwrap_or_else(|| "?".into());
        let name = if d.name.is_empty() { "?" } else { &d.name };
        println!("{:<name_width$}  {:<17}  {:>4}  {}", name, d.address, rssi, d.service);
    }
    println!("\nUse a device as ble:<name> (or a unique start of the name), e.g. raft fs ls ble:{}", found[0].name);
    Ok(())
}

#[cfg(not(feature = "ble"))]
fn scan(_timeout: u64, _ble: &BleArgs) -> CmdResult<()> {
    Err("this raftcli was built without the ble feature".into())
}
