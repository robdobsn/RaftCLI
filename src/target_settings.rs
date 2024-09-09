// target_settings.rs - RaftCLI: Target settings
// Rob Dobson 2024

// Define the schema for the user input
#[derive(Debug, Serialize, Deserialize, Clone)]
struct TargetSettings {
    sys_types: Vec<SysType>,
}
