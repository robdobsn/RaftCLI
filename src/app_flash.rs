use std::path::Path;
use crate::raft_cli_utils::utils_get_sys_type;
use espflash::cli::config;
use espflash::cli::connect;
use espflash::cli::ConnectArgs;

pub fn flash_raft_app(build_sys_type: &Option<String>, app_folder: String, port: String, baud: u32) -> Result<(), Box<dyn std::error::Error>> {

    // Get the app folder
    let app_folder_path = Path::new(&app_folder);

    // Determine the Systype to build
    let sys_type = utils_get_sys_type(build_sys_type, &app_folder);

    // Get the app build folder
    let build_folder = app_folder_path.join("build");

    // Get the app bin folder
    let bin_folder = build_folder.join(&sys_type);

    // Get the app bin file
    let bin_file_path = bin_folder.join(format!("{}.elf", &sys_type));

    // Check the app folder is valid
    println!("Flashing app sys_type {} app_folder {} port {} baud {} bin_file_path {:?}", 
                &sys_type, app_folder_path.display(), port, baud, bin_file_path);

    // Create an instance of ConnectArgs with specific values.
    let connect_args = ConnectArgs {
        baud: Some(baud), // Example baud rate
        port: Some(port.to_string()), // Example port name
        no_stub: false,
    };

    // Start flashing using espflash library
    let config = config::Config::default();

    let flasher = connect(
        &connect_args,
        &config
    );

    if flasher.is_err() {
        println!("Error: {:?}", flasher.err().unwrap());
        return Err(Box::new(std::io::Error::new(std::io::ErrorKind::Other, "Flash failed")));
    }

    print!("Flasher created");
    // let flasher = flasher.unwrap();
    // flasher.verify_minimum_revision(1)?;

    // let flash_result = espflash::flash(&bin_file_path, &port, baud);
    // if flash_result.is_err() {
    //     return Err(Box::new(std::io::Error::new(std::io::ErrorKind::Other, "Flash failed")));
    // }




    Ok(())
}
