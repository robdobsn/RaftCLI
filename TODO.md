# RaftCLI ToDo list

- default name the project after the selected folder if it is empty
- raft -f doesn't seem to work on wsl
- config ini or similar in proj dir
  - define serial port or at least filter
  - define other flags
  - define something to run initially like . ~/esp/esp-v/export.sh etc
  - define default systype
  - config file for default settings - maybe use platformio.ini when in that system?
- need a way to build with docker and then switch to idf - currently build with docker then run idf export.sh then raft run results in a one line display and then exit the program without doing anything
- add option to avoid programming file system flash if it hasn't changed since last programming
- consider whether to allow entry of ssid and password in CLI written to sdkconfig.defaults file or possibly to a config.ini type file that is .gitignored
- bug noticed on Mac with invalid chars immediately after boot
- change so logging is the default and -l disables
- test on mac and linux
- rethink the build process for ESP IDF and potentially other platforms
  - ESP IDF build dependencies not quite right for web ui - gets stuck saying can't build files that now don't exist due to dynamic naming of files and auto generating of build script based on folder contents
  - can it be made to work on platformio for arduino
  - can it be made to work on arduino IDE
  - what about platform io for ESP IDF
  - one idea is to put the main pre-build work into a python script or even a rust script - maybe in some cases this is invoked manually (such as Arduino IDE) - or in the platformio case a prebuild script can be specified in the library.json file
  - this script would do all the stuff that RaftProject.cmake script does
  - another post-build script might be needed too though - although maybe not? this would perhaps do what RaftGenFSImage.cmake does - though in fact this probably isn't necessary as long as arduinoIDE can be configured to write the FS image?
  - on platformio the configurations could maybe be managed by the platformio.ini file so and there could be an option in the raftcli to generate a platformio.ini file?
  - maybe there should be a raftcli prebuild function which runs this script?

## Fixed in 1.7.2
- ESP IDF 5.4.2
- pretty print JSON systype
- add devjson & devbin to publish and ws?
- if n is answered in raft new for BLE then Error evaluating condition: use_raft_ble_central: Variable identifier is not bound to anything by context: "use_raft_ble_central". Fixed.
- changed to use 1.23.1 RaftCore script RaftBootstrap.cmake

## Fixed in 1.6.6
- Change CMakeLists.txt to bootstrap version
- move RaftCoreApp raftCoreApp; outside main();
- add option for BLE settings on new scaffold
- include size of flash as a user input


## Fixed in 1.4.3
- add OTA update using curl or build-in rust TCP implementing HTTP Post
- add a command history to the terminal emulation
- move up to 5.3.1 ESP IDF - testing needed to ensure Raft works with it - serial output seemed different
- raft new - change false/true to Y / N and allow Y/y/N/n as answers to questions

## Fixed in 1.2.2
- changed default to ESP IDF 5.3

## Fixed in 1.2.0
- raft new - doesn't seem to honour no response on web server question

## Fixed in 1.1.6
- entering text into serial terminal seems to kill it sometimes
- move template MODULE_PREFIX to constexpr in header
- add .gitattributes
- if no docker or idf then silently stops

## Fixed in 1.1.1
- allow one letter cmds for monitor (m), run (r)
- add .gitattributes # Auto detect text files and perform LF normalization .. * text=auto
- handle reconnection automatically - if serial port disconnected then reconnected
- fix problem invalid string ... thread 'tokio-runtime-worker' panicked at src/serial_monitor.rs:179:68: Failed to read from RX stream: Custom { kind: Other, error: "Invalid String" }
- add monitor as option in Makefile?
- remove makefile generation???
- monitor seems to have incresing processor overhead over time on WSL - is this to do with logging?
- shift default ESP IDF to 5.2.1
- monitor is a problem - up arrow in terminal goes up a line
- allow one letter cmds for monitor (m), run (r)
- detect IDF environment and use docker only on windows/wsl and only if docker is present
- use --no_docker for nodocker and --docker for yesdocker
