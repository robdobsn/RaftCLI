// systype_config.rs - RaftCLI: SysType Configuration
// Rob Dobson 2024

use crate::raft_cli_utils::read_platform_ini;

// Define the configuration for the SysType
#[derive(Debug, Clone)]
pub struct SysTypeConfig {
    pub target_chip: String
}

// Extract systype info from the platform.ini file for the specified systype section 
pub fn systype_config_extract_systype_info(app_folder: String, sys_type: String) -> SysTypeConfig {

    // SysTypeConfig to return
    let mut sys_type_config = SysTypeConfig {
        target_chip: "".to_string()
    };

    // Read the platform.ini file
    let platform_ini = read_platform_ini(app_folder.clone());

    // Get the SysType section which is named [env::<sys_type>]
    if let Ok(ref platform_ini) = platform_ini {
        if let Some(target_chip) = platform_ini.get_from(Some(format!("env:{}", sys_type).as_str()), "target_chip") {
            sys_type_config.target_chip = target_chip.to_string();
        } else {
            // Check if there is a common section named [env]
            if let Some(systype_env) = platform_ini.get_from(Some("env"), "target_chip") {
                sys_type_config.target_chip = systype_env.to_string();
            }
        }            
    }
    // Return the SysTypeConfig
    sys_type_config

}