# Raft CLI (command-line interface)

Raft is an opinionated framework for developing embedded apps for the Espressif ESP32 family - see [further information on Raft](https://github.com/robdobsn/RaftCore)

This command-line application is used to scaffold, build, flash and monitor raft apps.

- [Creating a new raft app](#creating-a-new-raft-app)
- [Building a raft app](#building-a-raft-app)
- [Flashing firmware to a development board](#flashing-the-firmware-to-a-development-board)
- [Monitoring a serial port](#monitoring-a-serial-port)
- [Installation](#installation)
- [Scaffolding Questions](#scaffolding-questions)


## Creating a new raft app

```
$ raft new .
```
This creates a new raft app in the current folder. 

You will be asked a series of questions including the name of your app, target chip, etc. For details on the questions and what they mean, [refer to the Scaffolding Questions section below](#scaffolding-questions)

If you want to scaffold your app in a different folder then replace the . in the above command line to your choice of folder. Note that the app will be created directly in that folder (not in a sub folder) so if you want your app to be in a folder called ./dev/MyRustApp then you must specify this full path.

On completion you will have a folder structure like this:

```
├───components
│   └───MySysMod
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
```

This will build the raft app in the current folder using Docker. If your raft app has multiple SysTypes then you can define which SysType to build using the -s option.

If you don't want to use Docker for the build then you can use the -d option and, in this case, you will need to ensure that a correctly installed ESP IDF (Espressif's development environment) is present on the system. You can override the location of this ESP IDF using the -i option.

To perform a clean build use the -c option.

```
Build a raft app

Usage: raft build [OPTIONS] [APP_FOLDER]

Arguments:
  [APP_FOLDER]

Options:
  -s, --sys-type <SYS_TYPE>  System type to build
  -c, --clean                Clean the target folder
  -d, --no-docker            Don't use docker for build
  -i, --idf-path <IDF_PATH>  Full path to idf.py (when not using docker)
  -h, --help                 Print help
```

## Flashing the firmware to a development board

To program a development board, use:

```
raft run
```

This will first build the firmware (and all of the build options are availble as above), then it will flash the firmare to the Espressif processor and then start a serial monitor.

To specify the serial port to be used there is the -p option. You can also specify baud rate for flashing using -f option. This program generally uses the espressif esptool to do the actual work of flashing. If you need to specify the full path to this tool then use the -t option.

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
  -d, --no-docker                    Don't use docker for build
  -i, --idf-path <IDF_PATH>          Full path to idf.py (when not using docker)
  -p, --port <PORT>                  Serial port
  -b, --monitor-baud <MONITOR_BAUD>  Monitor baud rate
  -r, --no-reconnect                 Disable serial port reconnection when monitoring
  -n, --native-serial-port           Native serial port when in WSL
  -f, --flash-baud <FLASH_BAUD>      Flash baud rate
  -t, --flash-tool <FLASH_TOOL>      Flash tool (e.g. esptool)
  -l, --log                          Log serial data to file
  -g, --log-folder <LOG_FOLDER>      Folder for log files [default: ./logs]
  -h, --help                         Print help
```

## Monitoring a serial port

```
raft monitor
```

This starts the serial monitor, displaying serial output received from the device and sending keyboard commands to the device. To specify the serial port use the -p option and to specify the baud rate for monitoring use -b.

Logging of received serial data can be enabled using the -l option. This is very useful when debugging as it automatically names log files with their start date and time and provides a record of test runs when developing firmware. The folder ./logs is generally used for log files but this can be changed using the -g option. 

The -r option is used to suppress automatic reconnection of serial ports during serial monitoring. Normally the serial monitor remains running even if a development board is disconnected. This makes development easier as it is often necessary to reset or disconnect a development board and having to restart the serial monitor each time is a nuissance. But if required the -r option can be specified which will disable reconnection.

The -n option is only relevant when using Windows Subsystem for Linux (WSL). The normal behaviour when using WSL is that flashing and serial monitoring are done with Windows versions of the raftcli software. This is because WSL (specifically WSL2) doesn't have support for USB serial ports to be shared with the host operating system. Specifying -n causes the raftcli to use a linux to access the serial port. This will only work if you are using something like (USBIPD)[https://github.com/dorssel/usbipd-win].

```
Monitor a serial port

Usage: raft monitor [OPTIONS]

Options:
  -p, --port <PORT>                  Serial port
  -b, --monitor-baud <MONITOR_BAUD>  Baud rate
  -r, --no-reconnect                 Disable serial port reconnection when monitoring
  -n, --native-serial-port           Native serial port when in WSL
  -l, --log                          Log serial data to file
  -g, --log-folder <LOG_FOLDER>      Folder for log files [default: ./logs]
  -h, --help                         Print help
  ```

To exit the serial monitor press ESC

## Installation

Installation using the crates.io package (raftcli is an application writted in the Rust programming language) is by far the easiest option but the program can also be built from source code if you need to.

Firstly install Rust (since you are here I assume you are interested in embedded development and Rust may be in your future in any case :)

Follow the steps to [install rust](https://www.rust-lang.org/tools/install)

Then install the app with:

```
cargo install raftcli
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
| Project Version | version number in semver format, e.g. 1.2.3 |
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