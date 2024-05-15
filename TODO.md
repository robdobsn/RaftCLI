# RaftCLI ToDo list

- handle reconnection automatically - if serial port disconnected then reconnected
- add a command history to the terminal emulation
- consider whether to allow entry of ssid and password in CLI written to sdkconfig.defaults file or possibly to a config.ini type file that is .gitignored
- possibly include size of flash as a user input - issues with this are different flash types such as OCTAL in addition to size
- fix problem invalid string ... thread 'tokio-runtime-worker' panicked at src/serial_monitor.rs:179:68: Failed to read from RX stream: Custom { kind: Other, error: "Invalid String" }
- add monitor as option in Makefile?
- bug noticed on Mac with invalid chars immediately after boot
- remove makefile generation???
- monitor seems to have incresing processor overhead over time on WSL - is this to do with logging?
- config file for default settings - maybe use platformio.ini when in that system?
- shift default ESP IDF to 5.2.1
- change so logging is the default and -l disables

[] monitor is a problem - up arrow in terminal goes up a line

- rethink the build process for ESP IDF and potentially other platforms
-- ESP IDF build dependencies not quite right for web ui - gets stuck saying can't build files that now don't exist due to dynamic naming of files and auto generating of build script based on folder contents
-- can it be made to work on platformio for arduino
-- can it be made to work on arduino IDE
-- what about platform io for ESP IDF
-- one idea is to put the main pre-build work into a python script or even a rust script - maybe in some cases this is invoked manually (such as Arduino IDE) - or in the platformio case a prebuild script can be specified in the library.json file
-- this script would do all the stuff that RaftProject.cmake script does
-- another post-build script might be needed too though - although maybe not? this would perhaps do what RaftGenFSImage.cmake does - though in fact this probably isn't necessary as long as arduinoIDE can be configured to write the FS image?
-- on platformio the configurations could maybe be managed by the platformio.ini file so and there could be an option in the raftcli to generate a platformio.ini file?
-- maybe there should be a raftcli prebuild function which runs this script?

