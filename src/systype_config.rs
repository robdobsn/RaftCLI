// systype_config.rs - RaftCLI: SysType Configuration
// Rob Dobson 2024

use std::path::Path;

use crate::raft_cli_utils::read_platform_ini;

// Define the configuration for the SysType
#[derive(Debug, Clone)]
pub struct SysTypeConfig {
    pub target_chip: String,
    pub partition_table_file: String,
    pub sdkconfig_defaults_file: String
}

// Extract systype info from the platform.ini file for the specified systype section 
pub fn systype_config_extract_systype_info(app_folder: String, sys_type: String) -> SysTypeConfig {

    // SysTypeConfig to return
    let mut sys_type_config = SysTypeConfig {
        target_chip: "".to_string(),
        partition_table_file: "".to_string(),
        sdkconfig_defaults_file: "".to_string()
    };

    // Read the platform.ini file
    let platform_ini = read_platform_ini(app_folder.clone());

    // Get the SysType section which is named [env::<sys_type>]
    if let Ok(ref platform_ini) = platform_ini {

        // Get the target chip
        if let Some(target_chip) = platform_ini.get_from(Some(format!("env:{}", sys_type).as_str()), "target_chip") {
            // Use the target_chip from the specified section
            sys_type_config.target_chip = target_chip.to_string();
        } else if let Some(systype_env) = platform_ini.get_from(Some("env"), "target_chip") {
            // Use the target_chip from the common section
            sys_type_config.target_chip = systype_env.to_string();
        }

        // Get the partition table file
        if let Some(partition_table_file) = platform_ini.get_from(Some(format!("env:{}", sys_type).as_str()), "board_build.partitions") {
            // Use the partition_table_file from the specified section
            sys_type_config.partition_table_file = partition_table_file.to_string();
        } else if let Some(systype_env) = platform_ini.get_from(Some("env"), "board_build.partitions") {
            // Use the partition_table_file from the common section
            sys_type_config.partition_table_file = systype_env.to_string();
        }

        // Get the sdkconfig defaults file
        if let Some(sdkconfig_defaults_file) = platform_ini.get_from(Some(format!("env:{}", sys_type).as_str()), "sdkconfig_defaults") {
            // Use the sdkconfig_defaults_file from the specified section
            sys_type_config.sdkconfig_defaults_file = sdkconfig_defaults_file.to_string();
        } else if let Some(systype_env) = platform_ini.get_from(Some("env"), "sdkconfig_defaults") {
            // Use the sdkconfig_defaults_file from the common section
            sys_type_config.sdkconfig_defaults_file = systype_env.to_string();
        } else {
            // check if file <app_folder>/systypes/<systype>/sdkconfig.defaults exists
            let sdkconfig_defaults_relative = format!("systypes/{}/sdkconfig.defaults", sys_type);
            let sdkconfig_defaults_abs = format!("{}/{}", app_folder, sdkconfig_defaults_relative);
            let sdkconfig_defaults_path = Path::new(&sdkconfig_defaults_abs);
            if sdkconfig_defaults_path.exists() {
                sys_type_config.sdkconfig_defaults_file = sdkconfig_defaults_relative;
            } else {
                // Use the sdkconfig_defaults_file from the common section
                sys_type_config.sdkconfig_defaults_file = format!("systypes/Common/sdkconfig.defaults");
            }
        }
    }
    
    // Return the SysTypeConfig
    sys_type_config

}