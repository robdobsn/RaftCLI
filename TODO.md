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


