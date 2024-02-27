use std::path::Path;

pub fn flash_raft_app(sys_type: &Option<String>, app_folder: String, port: String, baud: u32) -> Result<(), Box<dyn std::error::Error>> {
    // Get the app folder
    let app_folder = Path::new(&app_folder);

    // Get the app name
    let app_name = app_folder.file_name().unwrap().to_str().unwrap();

    // Get the app build folder
    let build_folder = app_folder.join("build");

    // Get the app bin folder
    let bin_folder = build_folder.join("bin");

    // Get the app bin file
    let bin_file_path = bin_folder.join(app_name);

    println!("Flashing app sys_type {} app_folder {} port {} baud {} bin_file_path {:?}", 
                sys_type.as_ref().map(|s| s.as_str()).unwrap_or("default"), app_folder.display(), port, baud, bin_file_path);


    Ok(())
}
