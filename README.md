# Raft CLI (command-line interface)

> Note: installation is with 'cargo install raftcli' and not as listed in the sidebar [see installation section below](#installation)

Raft is an opinionated framework for developing embedded apps for the Espressif ESP32 family - see [further information on Raft](https://github.com/robdobsn/RaftCore)

This command-line application is used to scaffold, build, flash and monitor raft apps.

- [Installation](#installation)
- [Creating a new raft app](#creating-a-new-raft-app)
- [Building a raft app](#building-a-raft-app)
- [Flashing firmware to a development board](#flashing-the-firmware-to-a-development-board)
- [Monitoring a serial port](#monitoring-a-serial-port)
- [Scaffolding Questions](#scaffolding-questions)

## Installation

Installation using the online service crates.io (raftcli is an application written in the Rust programming language) is by far the easiest option but the program can also be built from source code if you need to.

Firstly install Rust (since you are here I assume you are interested in embedded development and Rust may be in your future in any case :) follow these steps to [install rust](https://www.rust-lang.org/tools/install)

Then install the app with:

```
cargo install raftcli
```

If you are using a linux OS and encountering issues then please make sure the following system dependencies are installed:

```sh
sudo apt-get update
sudo apt-get install pkg-config libudev-dev
```

There are some addition things you'll need to have on your system to support building and flashing raft apps:

### Build using Docker

The default option for building a raft app is to use [Docker](https://docs.docker.com/get-docker/) so please go ahead and install that if you don't have it already.

The other thing you'll need in this scenario is a way to flash the ESP32 family chip. The simplest way to install this is using python's pip package. Make sure python and pip are installed first (you can type `python3 -m pip --version` or `python -m pip --version` if you are on Windows). If pip isn't installed then [install python and pip](https://www.python.org/downloads/). Install the esptool with:

```
python3 -m pip install esptool
```

> Note: you may need to use python instead of python3 on Windows.

### Build using ESP IDF

Alternatively you can [install the Espressif ESP IDF](https://docs.espressif.com/projects/esp-idf/en/stable/esp32/get-started/index.html). Make sure all of the requirements are installed correctly as I find the Espressif installation docs to be a bit unclear. Also, if installing an ESP IDF from the [releases page on github](https://github.com/espressif/esp-idf/releases), ensure that you install the tools by changing to the ESP IDF folder and running ./install.sh or similar commands on different OSs - see [install-scripts](https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-guides/tools/idf-tools.html#install-scripts).

You will also need to make sure you run the raft command line interface program in a shell with the IDF environment installed. You can do this on linux/mac using a command like `. ~/esp/esp-idf-v5.3/export.sh` or similar based on where the esp idf got installed and what version it is. Another option, and one that works on Windows, is use the [Espressif VS Code extension](https://github.com/espressif/vscode-esp-idf-extension) which handles the shell environment for you. Otherwise, on Windows and depending on how you installed the ESP IDF, you may need to use a shortcut that runs the ESP IDF shell. 

In this case use the -d option, i.e. `raft run -d` or `raft build -d` to disable the use of Docker.

## Creating a new raft app

```
$ raft new .
OR
$ raft n .
```
This creates a new raft app in the current folder. 

You will be asked a series of questions including the name of your app, target chip, etc. For details on the questions and what they mean, [refer to the Scaffolding Questions section below](#scaffolding-questions)

If you want to scaffold your app in a different folder then replace the . in the above command line to your choice of folder. Note that the app will be created directly in that folder (not in a sub folder) so if you want your app to be in a folder called ./dev/MyRustApp then you must specify this full path.

On completion you will have a folder structure like this:

```
├───components
│   └───MainSysMod
├───main
└───systypes
    ├───Common
    │   ├───FSImage
    │   └───WebUI
    └───SysTypeMain
```

Additional options are available ...

```
Create a new raft app

Usage: raft new [OPTIONS] [BASE_FOLDER]

Arguments:
  [BASE_FOLDER]

Options:
  -c, --clean  Clean the target folder
  -h, --help   Print help
```
## Building a raft app

To build an existing raft app use:

```
raft build
OR
raft b
```

This will build the raft app in the current folder using Docker (unless you are in a prompt with the ESP IDF already sourced in which case ESP IDF will be used natively). If your raft app has multiple SysTypes then you can define which SysType to build using the -s option.

If you don't want to use Docker for the build then you can use the no-docker option (see below) and, in this case, you will need to ensure that a correctly installed ESP IDF (Espressif's development environment) is present on the system. You can override the location of this ESP IDF using the -i option.

To perform a clean build use the -c option.

```
Build a raft app

Usage: raft build [OPTIONS] [APP_FOLDER]

Arguments:
  [APP_FOLDER]

Options:
  -s, --sys-type <SYS_TYPE>  System type to build
  -c, --clean                Clean the target folder
  -n, --clean-only           Clean only
      --docker               Use docker for build
      --no-docker            Do not use docker for build
  -i, --idf-path <IDF_PATH>  Full path to idf.py (when not using docker)
  -h, --help                 Print help
```

## Build and flash firmware to a development board

To build and flash a development board, use:

```
raft run
OR
raft r
```

This will first build the firmware (and all of the build options are availble as above), then it will flash the firmare to the Espressif processor and then start a serial monitor.

If the serial port is not specified (with the -p option) then the most likely suitable serial port will be tried. You can also specify baud rate for flashing using -f option. This program generally uses the espressif esptool to do the actual work of flashing. If you need to specify the full path to this tool then use the -t option.

Once the flashing process is complete the monitoring function will be started to view the output from the serial port. The options for this are described below and can be used both run and monitor commands.

To exit the serial monitor press ESC

```
Build, flash and monitor a raft app

Usage: raft run [OPTIONS] [APP_FOLDER]

Arguments:
  [APP_FOLDER]  

Options:
  -s, --sys-type <SYS_TYPE>          System type to build
  -c, --clean                        Clean the target folder
      --docker                       Use docker for build
      --no-docker                    Do not use docker for build
  -i, --idf-path <IDF_PATH>          Full path to idf.py (when not using docker)
  -p, --port <PORT>                  Serial port
  -b, --monitor-baud <MONITOR_BAUD>  Monitor baud rate
  -r, --no-reconnect                 Disable serial port reconnection when monitoring
  -n, --native-serial-port           Native serial port when in WSL
  -f, --flash-baud <FLASH_BAUD>      Flash baud rate
  -t, --flash-tool <FLASH_TOOL>      Flash tool (e.g. esptool)
  -l, --log                          Log serial data to file
  -g, --log-folder <LOG_FOLDER>      Folder for log files [default: ./logs]
  -v, --vid <VID>                    Vendor ID
  -h, --help                         Print help
```

## Monitoring a serial port

```
raft monitor
OR
raft m
```

This starts the serial monitor, displaying serial output received from the device and sending keyboard commands to the device. If a serial port isn't specified (with the -p option) then the most likely suitable port will be used. To specify the baud rate for monitoring use -b.

When in the serial monitor up-arrow and down-arrow show prior command history (as when using bash linux shell).

Logging of received serial data can be enabled using the -l option. This is very useful when debugging as it automatically names log files with their start date and time and provides a record of test runs when developing firmware. The folder ./logs is generally used for log files but this can be changed using the -g option. 

The -r option is used to suppress automatic reconnection of serial ports during serial monitoring. Normally the serial monitor remains running even if a development board is disconnected. This makes development easier as it is often necessary to reset or disconnect a development board and having to restart the serial monitor each time is a nuissance. But if required the -r option can be specified which will disable reconnection.

The -n option is only relevant when using Windows Subsystem for Linux (WSL). The normal behaviour when using WSL is that flashing and serial monitoring are done with Windows versions of the raftcli software. This is because WSL (specifically WSL2) doesn't have support for USB serial ports to be shared with the host operating system. Specifying -n causes the raftcli to use a linux to access the serial port. This will only work if you are using something like (USBIPD)[https://github.com/dorssel/usbipd-win].

```
Monitor a serial port

Usage: raft.exe monitor [OPTIONS] [APP_FOLDER]

Arguments:
  [APP_FOLDER]

Options:
  -p, --port <PORT>                  Serial port
  -b, --monitor-baud <MONITOR_BAUD>  Baud rate
  -r, --no-reconnect                 Disable serial port reconnection when monitoring
  -n, --native-serial-port           Native serial port when in WSL
  -l, --log                          Log serial data to file
  -g, --log-folder <LOG_FOLDER>      Folder for log files [default: ./logs]
  -v, --vid <VID>                    Vendor ID
  -h, --help                         Print help
  ```

To exit the serial monitor press ESC

## Listing serial ports

To list available serial ports use:

```
raft ports
OR
raft p
```

Manage serial ports

Usage: raft ports [OPTIONS]

Options:
  -p, --port <PORT>                      Port pattern
  -v, --vid <VID>                        Vendor ID
  -d, --pid <PID>                        Product ID
      --manufacturer <MANUFACTURER>      Manufacturer
      --serial <SERIAL>                  Serial number
      --product <PRODUCT>                Product name
  -i, --index <INDEX>                    Index
  -D, --debug                            Debug mode
      --preferred-vids <PREFERRED_VIDS>  Preferred VIDs (comma separated list)
  -n, --native-serial-port               Native serial port when in WSL
  -h, --help                             Print help

When using WSL, the ports command automatically delegates to the Windows version (raft.exe) to access USB serial ports unless the -n flag is specified.

## Flash firmware to a development board (without rebuilding)

To flash firmware, use:

```
raft flash
OR
raft f
```

This will flash the firmare to the Espressif processor.

If the serial port is not specified (with the -p option) then the most likely suitable serial port will be tried. You can also specify baud rate for flashing using -f option. This program generally uses the espressif esptool to do the actual work of flashing. If you need to specify the full path to this tool then use the -t option.

```
Flash firmware to the device

Usage: raft.exe flash [OPTIONS] [APP_FOLDER]

Arguments:
  [APP_FOLDER]

Options:
  -s, --sys-type <SYS_TYPE>      System type to flash
  -p, --port <PORT>              Serial port
  -n, --native-serial-port       Native serial port when in WSL
  -f, --flash-baud <FLASH_BAUD>  Flash baud rate
  -t, --flash-tool <FLASH_TOOL>  Flash tool (e.g. esptool)
  -v, --vid <VID>                Vendor ID
  -h, --help                     Print help
```

## OTA (Over-the-air) Update Firmware (using WiFi/Ethernet connection)

To use OTA updates the device must be connected to a WiFi or Ethernet network and the IP address (or hostname) of the device must be known.

```
raft ota
OR
raft o
```

A connection (TCP) is made to the device and the standard HTTP POST protocol with form data is used to send the new firmware to the device. If required the -c option can be used which forces the use of the curl application to send the data - otherwise sending is done using the rust TcpStream mechanism which also permits rate and progress information to be shown.

Only the main binary of the application is written. The file-system and other partitions on the device including non-volatile storage are not affected by this operation.

```
Over-the-air update

Usage: raft.exe ota [OPTIONS] <IP_ADDR> [APP_FOLDER]

Arguments:
  <IP_ADDR>
  [APP_FOLDER]

Options:
  -p, --ip-port <IP_PORT>    IP Port
  -s, --sys-type <SYS_TYPE>  System type to ota update
  -c, --use-curl             Use curl for OTA
  -h, --help                 Print help
```

### Build from source

If you want to build this app from source code then firstly [install rust](https://www.rust-lang.org/tools/install)

Clone the repo:

```
git clone https://github.com/robdobsn/RaftCLI
cd RaftCLI
cargo install --path .
```

This will build and install the app into the binary executables folder that cargo uses.

## Scaffolding Questions

The following questions are asked to complete the scaffolding from template files:

| Question | Explanation |
| -- | -- |
| Project Name | name for your project |
| Target Chip | e.g. esp32, esp32s3 or esp32c3 |
| SysType | the name of the main SysType (or system type) - SysTypes, for instance, allow a project to target different hardware - set the name for the main SysType that you want to create here - additional SysTypes are added manually |
| ESP IDF Version | the version of the ESP IDF to use to build the app |
| Create User SysMod | Select true to create a SysMod for the main part of your application's code - SysMods are a key concept in raft apps as they allow user code to be managed like an Arduino app with setup() and loop() functions |
| User SysMod Class | If you answered true above then you will be asked for the name you want to give to your app's main SysMod |
| User SysMod Name | A SysMod can be given a different name from its class - so either enter the same name used for the Class here or give it a different name |
| Raft Core git tag | RaftCore is the core element of the raft framework. Specify which version of RaftCore to use here. The default will be the latest version - which is called main |
| Use RaftSysMods | RaftSysMods are building blocks that help build a raft application. For instance WiFi, MQTT, etc. Select true to enable these in your application |
| RaftSysMods git tag | The git tag of the RaftSysMods to use - defaults to main which is the latest version |
| Use RaftWebServer | Select true to enable the raft Web Server |
| RaftWebServer git tag | Git tag of the RaftWebServer - main is the latest version |


## Template App Details

New raft apps are "scaffolded" using template information in the raft_templates folder.

The handlebars templating library is used to fill in the gaps in the templates based on the answers to questions asked when running "raft new".

In addition to generating source code, build files are generated for various build scenarios including:

* building natively on linux
* building using docker on mac, linux and windows

The best way to build a raft application depends on your OS and requirements. I primarily use a Windows machine with Ubuntu in WSL (google this if you are not familiar with running linux with the Windows Subsystem for Linux).  I find this is the best combination of speed and convenience and the Makefile generated by this app works well in that scenario when run from a WSL prompt.