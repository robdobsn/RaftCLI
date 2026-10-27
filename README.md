# Raft CLI (command-line interface)

> Note: installation is with 'cargo install raftcli' and not as listed in the sidebar [see installation section below](#installation)

Raft is an opinionated framework for developing embedded apps for the Espressif ESP32 family - see [further information on Raft](https://github.com/robdobsn/RaftCore)

This command-line application is used to scaffold, build, flash and monitor raft apps, and to manage the files and
device type records on a running device over HTTP, WebSocket, USB serial or BLE.

- [Raft CLI (command-line interface)](#raft-cli-command-line-interface)
  - [Installation](#installation)
    - [Linux/WSL prerequisites](#linuxwsl-prerequisites)
    - [Build using Docker](#build-using-docker)
    - [Build using ESP IDF](#build-using-esp-idf)
  - [Troubleshooting](#troubleshooting)
    - [cargo install fails on Linux/WSL (libudev-sys / pkg-config)](#cargo-install-fails-on-linuxwsl-libudev-sys--pkg-config)
  - [Creating a new raft app](#creating-a-new-raft-app)
  - [Building a raft app](#building-a-raft-app)
  - [Build and flash firmware to a development board](#build-and-flash-firmware-to-a-development-board)
  - [Monitoring a serial port](#monitoring-a-serial-port)
  - [Listing serial ports](#listing-serial-ports)
  - [BLE devices](#ble-devices)
  - [Local development libraries](#local-development-libraries)
  - [Flash firmware to a development board (without rebuilding)](#flash-firmware-to-a-development-board-without-rebuilding)
  - [Run esptool directly](#run-esptool-directly)
  - [OTA (Over-the-air) Update Firmware (using WiFi/Ethernet connection)](#ota-over-the-air-update-firmware-using-wifiethernet-connection)
  - [Managing files on a device](#managing-files-on-a-device)
  - [Device type records](#device-type-records)
  - [Remote Debug Console](#remote-debug-console)
  - [Persistent Settings](#persistent-settings)
    - [Build from source](#build-from-source)
  - [Scaffolding Questions](#scaffolding-questions)
  - [Template App Details](#template-app-details)
  - [Appendix: Finding local Git changes](#appendix-finding-local-git-changes)

## Installation

Installation using the online service crates.io (raftcli is an application written in the Rust programming language) is by far the easiest option but the program can also be built from source code if you need to.

Firstly install Rust (since you are here I assume you are interested in embedded development and Rust may be in your future in any case :) follow these steps to [install rust](https://www.rust-lang.org/tools/install)

### Linux/WSL prerequisites

On Linux (including WSL), install these system packages **before** running `cargo install raftcli`:

```sh
sudo apt-get update
sudo apt-get install -y pkg-config libudev-dev libdbus-1-dev
```

Why this is needed: `raftcli` depends on `serialport`, which on Linux uses `libudev` via `pkg-config` at build time,
and on `btleplug` (for BLE devices), which uses D-Bus to talk to BlueZ.

Then install the app with:

```
cargo install raftcli
```

If the install fails to compile because of a problem in one of the crates that raftcli depends on, use `cargo install raftcli --locked` instead. By default `cargo install` uses the newest compatible version of every dependency, whereas `--locked` uses exactly the versions that the release was built and tested with.

To update to a newer release run `cargo install raftcli` again. `raft new`, `build`, `run`, `libs` and `update-project` check crates.io at most once a day (using `curl`) and print a note when a newer release is available. Set `RAFT_NO_UPDATE_CHECK=1` to turn this off.

If you are using a Linux OS and see an install failure like:

- `failed to run custom build command for libudev-sys`
- `The pkg-config command could not be found`

see the [Troubleshooting](#troubleshooting) section below for exact recovery steps.

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

The recommended way to install the ESP IDF is now the [Espressif Installation Manager (EIM)](https://docs.espressif.com/projects/idf-im-ui/en/latest/), for example `eim install -i v6.1`. RaftCLI finds ESP IDF versions installed by EIM automatically (from EIM's `eim_idf.json` manifest) as well as ones installed the traditional way in `~/esp` (Linux/Mac) or `C:\Espressif\frameworks` (Windows). So the simplest way to build is `raft build -i` which finds an installed ESP IDF of the version the app requires, from any shell, with no need to set up the ESP IDF environment first. See [Which ESP IDF version is used](#which-esp-idf-version-is-used).

Alternatively you can run the raft command line interface program in a shell with the IDF environment installed. You can do this on linux/mac using a command like `. ~/esp/esp-idf-v6.0/export.sh` or similar based on where the esp idf got installed and what version it is. Another option, and one that works on Windows, is use the [Espressif VS Code extension](https://github.com/espressif/vscode-esp-idf-extension) which handles the shell environment for you. Otherwise, on Windows and depending on how you installed the ESP IDF, you may need to use a shortcut that runs the ESP IDF shell. 

In this case use the `--no-docker` option, i.e. `raft run --no-docker` or `raft build --no-docker` to disable the use of Docker. If the ESP IDF in the shell is not the version the app requires then RaftCLI looks for an installed one which is.

### Which ESP IDF version is used

The ESP IDF version required to build a SysType is found in this order:

1. the `--idf-version` option e.g. `raft build --idf-version 6.1`
2. `set(ESP_IDF_VERSION "6.1")` in `systypes/<SysType>/features.cmake` - so different SysTypes in the same app can be built with different versions
3. `set(ESP_IDF_VERSION "6.1")` in `systypes/Common/features.cmake` - the default for all SysTypes (this is where `raft new` puts it)
4. the version in the `Dockerfile` (`FROM espressif/idf:v6.1`) - this is how apps created with earlier versions of RaftCLI specify it and continues to work
5. RaftCLI's default version (currently 6.1)

The `set(ESP_IDF_VERSION ...)` line must contain a literal version on a single line because RaftCLI reads it without running CMake. In a SysType's `features.cmake` place it after the `include` of the Common file.

The same version is used for local and Docker builds:

- Local builds use an installed ESP IDF of that version. The `-e` option specifies one explicitly, either as the path of an ESP IDF folder or the name of an EIM installation (as shown by `eim list`) e.g. `raft build -e v6.1`, and this choice is remembered for later builds. If EIM's manifest is not in the default location use `--eim-json <path to eim_idf.json>` or set the `RAFT_EIM_IDF_JSON` environment variable.
- Docker builds use the `espressif/idf` image with that tag. Apps created by `raft new` have a `Dockerfile` that takes the version as a build argument (`ARG ESP_IDF_VERSION=v<version>` / `FROM espressif/idf:${ESP_IDF_VERSION}`) and RaftCLI supplies the required version when it builds the image, overriding the default. The default is set by `raft new` to match `features.cmake` so the `Dockerfile` can also be built directly with `docker build`. If a `Dockerfile` does contain a literal version and a different one is required then RaftCLI leaves the `Dockerfile` alone and builds the image from a generated copy in `build/raft_docker/<SysType>/Dockerfile`. Images are tagged `raftbuilder:idf-<version>`; these are large so you may want to remove ones no longer needed (`docker image ls raftbuilder`).

The ESP IDF component manager's lock file (which records the versions of managed components such as mdns and littlefs, together with the ESP IDF version and target chip they were chosen for) is kept per SysType, in `systypes/<SysType>/dependencies.lock`, in any app which sets `ESP_IDF_VERSION`. Otherwise SysTypes which use different ESP IDF versions (or different chips) would each invalidate a shared `dependencies.lock` every time you switched between them. Commit these files if you want repeatable builds. An app which doesn't set `ESP_IDF_VERSION` keeps using `dependencies.lock` in the app folder as before, unless a `dependencies.lock` exists in the SysType folder (so an older app can opt in by moving its lock file there). This needs a version of RaftCore which supports it.

When the required version for a SysType changes RaftCLI deletes that SysType's build folder since a build folder can't be reused with a different ESP IDF. With a version of RaftCore that supports it, building with a different ESP IDF version from the one required (for example by running `idf.py` directly in the wrong environment) is reported as an error by CMake.

## Troubleshooting

### cargo install fails on Linux/WSL (libudev-sys / pkg-config)

If `cargo install raftcli` fails with errors such as:

- `failed to run custom build command for libudev-sys`
- `The pkg-config command could not be found`

install the required system packages and retry:

```sh
sudo apt-get update
sudo apt-get install -y pkg-config libudev-dev libdbus-1-dev
cargo install raftcli
```

Equivalent packages on other distributions:

- Fedora/RHEL: `sudo dnf install pkgconf-pkg-config systemd-devel dbus-devel`
- Arch: `sudo pacman -S pkgconf systemd dbus`
- Alpine: `sudo apk add pkgconfig eudev-dev dbus-dev`

If BLE devices aren't needed, `cargo install raftcli --no-default-features` builds without BLE (and without D-Bus).

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
    └───RaftSys
```

At the end a summary shows what will be made: the hardware, the partition table, and an estimate of the app's size
and how much of its app slot is left for your own code (with suggestions if there's little room). The answers are
saved in the app as `.raftnew.json`.

Additional options are available ...

```
Create a new raft app

Usage: raft new [OPTIONS] [APPLICATION_FOLDER]

Arguments:
  [APPLICATION_FOLDER]  Path to the application folder

Options:
  -c, --clean               Clean the target folder
  -d, --defaults            Don't prompt - use the default answer for every question (see also --set)
      --set <KEY=VALUE>     Answer a question without prompting e.g. --set target_chip=esp32c6 (can be repeated)
      --advanced            Also ask the advanced questions (SysType name, SysMod name, main task core, ESP-IDF version)
      --answers <FILE>      Use the answers saved in another app's .raftnew.json (--set overrides them)
  -p, --port <PORT>         Serial port of the board to detect the hardware from (default: the most likely one)
  -n, --native-serial-port  Native serial port when in WSL (e.g. attached with usbipd) rather than a Windows one through raft.exe
  -h, --help                Print help
```

To create an app without being asked any questions (for instance from a script) use `--defaults`, optionally with
`--set` to answer specific questions. The keys are in [Scaffolding Questions](#scaffolding-questions). For example:

```
raft new MyApp --defaults --set board=um-feathers3 --set connectivity=ble
raft new MyApp --defaults --set module=ESP32-S3-WROOM-1-N8R2 --set i2c_sensors=extended
raft new MyApp --defaults --set target_chip=esp32c6 --set connectivity=wifi
raft new MyApp --defaults -p COM4
```

With `--defaults`, the hardware is detected from the board on `--port` if one is given (and `raft new` stops if it
can't be), otherwise it is the chip's safe defaults.

The keys of earlier RaftCLI versions still work: `use_raft_ble`, `use_raft_ble_peripheral`, `use_raft_ble_central`,
`use_raft_i2c`, `use_raft_core_dev_types` and `flash_size_for_partition_table` are translated to the questions that
replace them.

### The hardware

The first question after the project name is the hardware, which sets the chip, flash size, PSRAM and I2C pins:

- **Detect it from a connected board.** Offered for the port given with `-p/--port`, or when a serial port with an
  Espressif chip is found (the most likely one, if there are several). `esptool flash-id`
  reports the chip, its flash size and any PSRAM in the chip's package (PSRAM a module or board adds is asked for).
  Under WSL this uses Windows `raft.exe`, or the Windows esptool on the COM port; with `-n`, a port attached to WSL
  (e.g. with usbipd) and the Linux esptool. `raft detect` does the same on its
  own (`--json` for scripts).
- **A dev board** from a list (`--set board=<id>`): the preset gives everything, including the board's I2C
  connector pins. The boards are in `src/new_data/boards.json` - add one with a PR.
- **An Espressif module** by its ordering code (`--set module=ESP32-S3-WROOM-1-N8R2`): `N<flash MB>R<PSRAM MB>` gives
  the flash and PSRAM, the family gives the chip, the PSRAM mode (octal for R8 on the ESP32-S3) and pins the module
  uses itself, which can't be used for I2C. The families are in `src/new_data/modules.json`.
- **A chip, or "I don't know"** (`--set target_chip=..., flash_size=..., psram=...`): safe defaults (4 MB flash, no
  PSRAM) that run on almost any board with that chip. The summary says what was assumed.

### App types

The **App type** question sets the defaults of the later questions, and some generated settings, for a kind of app.
Every later question still works as usual, so a type is a starting point: `--set` answers and saved answers win
over its defaults.

| Type | Defaults it sets | Generated settings |
|-|-|-|
| `general` | The questions' own defaults | What `raft new` has always written |
| `battery` | connectivity `wifi`, no web server, no I2C | With WiFi: NetMan starts WiFi on demand (`"startMode": "onDemand"` - the app starts it with `{"cmd":"wifiStart"}`), connects straight to the remembered access point (`wifiSTAFastReconnect`), no access point or mDNS, and on the esp32c5 2.4 GHz only (`wifiBand`); RaftCore's `wifi-fast-connect` sdkconfig fragment (`RAFT_SDKCONFIG_FRAGMENTS` in `systypes/Common/features.cmake`). With BLE: a 1 s advertising interval. The generated README says how to set the per-device values (`w`, `wifiip`, the MQTT broker) |

Measured on an ESP32-C5 (RaftCore `devdocs/wifi-fast-connect-and-on-demand.md`): a wake from deep sleep to an IP
address takes 15.3 s with ESP-IDF's defaults and about 0.3 s with the battery settings and a static IP.

`raft update-project --set app_type=battery` on an existing app changes the generated settings (the NetMan block,
the fragments line) but not answers already saved - pass those too (e.g. `--set use_raft_webserver=false`).

Without I2C (`i2c_sensors` `none`) the device manager isn't built in (`set(RAFT_DEVICE_MANAGER OFF)`, about 100 KB of
flash), for every type. The plan for further types (high-rate publishing, sensor hub) is in
`devdocs/raft-new-app-types-proposal.md`.

### Changing answers later

`raft update-project --set <key=value>` changes one of raft new's answers (saved in `.raftnew.json`) and makes the
files that depend on it again - for example `raft update-project --set flash_size=8` after finding the board has 8 MB
of flash. Files that haven't been edited since they were generated are updated; an edited file is left as it is and
the new version is written beside it (`<file>.raftnew`) to merge by hand. Do a clean build afterwards (`raft build -c`)
when the partition table or sdkconfig changes.

## Building a raft app

To build an existing raft app use:

```
raft build
OR
raft b
```

This will build the raft app in the current folder using Docker (unless you are in a prompt with the ESP IDF already sourced in which case ESP IDF will be used natively). If your raft app has multiple SysTypes then you can define which SysType to build using the -s option.

If you don't want to use Docker for the build then you can use the no-docker option (see below) and, in this case, you will need to ensure that a correctly installed ESP IDF (Espressif's development environment) is present on the system. You can ask RaftCLI to find a local ESP IDF of the required version (including ones installed with the Espressif Installation Manager) using the -i option, or specify the ESP IDF using the -e option (which also selects a local build). See [Which ESP IDF version is used](#which-esp-idf-version-is-used).

To perform a clean build use the -c option.

```
Build a raft app

Usage: raft build [OPTIONS] [APPLICATION_FOLDER]

Arguments:
  [APPLICATION_FOLDER]  Path to the application folder

Options:
  -s, --sys-type <SYS_TYPE>          System type to build
  -c, --clean                        Clean the target folder
  -n, --clean-only                   Clean only
      --docker                       Use docker for build
      --no-docker                    Do not use docker for build
  -i, --idf-local-build              Find and use a local ESP IDF of the required version (see ESP_IDF_VERSION in features.cmake)
  -e, --esp-idf-path <ESP_IDF_PATH>  Path to ESP IDF folder, or name of an EIM installation, for local build (when not using docker)
      --idf-version <VERSION>        ESP IDF version to build with (overrides ESP_IDF_VERSION in features.cmake and the Dockerfile)
      --eim-json <FILE>              Path to the Espressif Installation Manager eim_idf.json (if not in the default location)
  -h, --help                         Print help
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
  -i, --idf-local-build              Find and use a local ESP IDF of the required version (see ESP_IDF_VERSION in features.cmake)
  -e, --esp-idf-path <ESP_IDF_PATH>  Path to ESP IDF folder, or name of an EIM installation, for local build (when not using docker)
      --idf-version <VERSION>        ESP IDF version to build with (overrides ESP_IDF_VERSION in features.cmake and the Dockerfile)
      --eim-json <FILE>              Path to the Espressif Installation Manager eim_idf.json (if not in the default location)
  -p, --port <PORT>                  Serial port
  -o, --ip-addr <IP_ADDR>            IP address or hostname for OTA flashing
  -b, --monitor-baud <MONITOR_BAUD>  Monitor baud rate
  -r, --no-reconnect                 Disable serial port reconnection when monitoring
  -n, --native-serial-port           Native serial port when in WSL
  -f, --flash-baud <FLASH_BAUD>      Flash baud rate
  -t, --flash-tool <FLASH_TOOL>      Flash tool (e.g. esptool)
  -l, --log                          Log serial data to file
  -g, --log-folder <LOG_FOLDER>      Folder for log files [default: ./logs]
  -v, --vid <VID>                    Vendor ID
      --no-fs                        Skip flashing the file system image
      --fs                           Flash the file system image (overrides saved --no-fs)
      --rx-timestamps <MODE>         Prefix received lines with wall-clock time: 'first' (on first byte) or 'eol' (on newline)
      --agent-stream                 Stream serial data to stdout without terminal UI for automation
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

The -n option is only relevant when using Windows Subsystem for Linux (WSL). The normal behaviour when using WSL is that flashing and serial monitoring are done with Windows versions of the raftcli software. This is because WSL (specifically WSL2) doesn't have support for USB serial ports to be shared with the host operating system. Specifying -n causes raftcli to use Linux to access the serial port. This will only work if you are using something like [USBIPD](https://github.com/dorssel/usbipd-win).

```
Monitor a serial port

Usage: raft monitor [OPTIONS] [APPLICATION_FOLDER]

Arguments:
  [APPLICATION_FOLDER]  Path to the application folder

Options:
  -p, --port <PORT>                  Serial port
  -b, --monitor-baud <MONITOR_BAUD>  Baud rate
  -r, --no-reconnect                 Disable serial port reconnection when monitoring
  -n, --native-serial-port           Native serial port when in WSL
  -l, --log                          Log serial data to file
  -g, --log-folder <LOG_FOLDER>      Folder for log files [default: ./logs]
  -v, --vid <VID>                    Vendor ID
      --rx-timestamps <MODE>         Prefix received lines with wall-clock time: 'first' (on first byte) or 'eol' (on newline)
      --agent-stream                 Stream serial data to stdout without terminal UI for automation
  -h, --help                         Print help
```

To exit the serial monitor press ESC

For automated testing or AI-agent workflows, use `--agent-stream`. This keeps regular monitor behaviour unchanged, but disables the interactive terminal UI and writes serial data directly to stdout. Status and reconnect messages are written to stderr.

## Listing serial ports

To list available serial ports use:

```
raft ports
OR
raft p
```

Manage serial ports

```
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
```

When using WSL, the ports command automatically delegates to the Windows version (raft.exe) to access USB serial ports unless the -n flag is specified.

## BLE devices

`raft fs` and `raft devtypes` can reach a Raft device over BLE, through its RICREST BLE service (RaftSysMods
`BLEMan`). **BLE has been tested on Windows (including from WSL) only so far;** it builds on macOS and Linux but
hasn't been tried there. Name the device as `ble:<name>`, where the name is the one it advertises, the start of it (if only one
device matches), or its address. `ble:` on its own uses the only Raft device in range.

To see the Raft devices in range:

```
raft ble scan [-t <seconds>]
```

```
NAME                     ADDRESS            RSSI  SERVICE
Axiom018_2cc23e          DC:B4:D9:2C:C2:3E   -46  aa76677e-9cfd-4626-a510-0d305be57c8d
```

Then, for example:

```
raft fs ls ble:Axiom018
raft fs put ble:Axiom018 config.json
```

- The first connection scans for the device (up to 10 s, usually 1-3 s). On Windows the address is remembered, so
  later connections skip the scan.
- Uploads run at about 15-25 KB/s and downloads at about 5-8 KB/s.
- **RaftCLI always releases the link when it finishes** (by sending the device `bledisconnect`), including on errors
  and Ctrl-C, because Windows otherwise keeps the BLE link up after a program exits.
- If RaftCLI is killed without the chance to do that, Windows ends the link itself about 25 s later, and a new
  RaftCLI run in the meantime reuses the held link. Devices with RaftSysMods 1.26 or later keep advertising while a
  link is held (they allow two connections), so other computers can still find them. Older firmware stops
  advertising while connected: if such a device that is in range can't be found, it is probably still held - type
  `bledisconnect` in `raft monitor`, open `http://<device>/api/bledisconnect` over WiFi, or restart it.
- Devices advertising other UUIDs can be reached with `--ble-service`, `--ble-cmd` and `--ble-resp`; `--ble-mtu`
  overrides the MTU if the one reported is wrong.
- WSL has no Bluetooth, so in WSL BLE commands (including `raft ble scan`) are handed to the Windows `raft.exe`, as
  serial commands are.
- On Linux, BlueZ must be running and the user may need to be in the `bluetooth` group. On macOS the terminal asks
  for Bluetooth permission the first time.

## Local development libraries

Raft projects can use local copies of Raft libraries from a `raftdevlibs` folder in the project root. The build uses `raftdevlibs/<LibraryName>` in place of the version in `RAFT_COMPONENTS` (in `features.cmake`), which is useful when developing or debugging a library. A library in `raftdevlibs` can be:

- a **clone** made for this project with `raft l fetch`, or
- a **link** (a symlink, or a junction on Windows) to a repo you work on elsewhere, made with `raft l link`. Several projects can link to the same repo.

The `raftdevlibs` folder is in `.gitignore`, so it is never committed.

| Command | Use |
| -- | -- |
| `raft l` or `raft l status` | Show each library in `raftdevlibs` (clone or link and where it points, pinned version, branch, commit, ahead/behind, uncommitted changes) and the libraries the build fetches. `--fetch` refreshes from the remotes first |
| `raft l fetch` (`raft l f`) | Clone libraries, or update existing clones (`git fetch`, checkout of `--branch`, fast-forward pull) |
| `raft l pull` (`raft l p`) | Fast-forward pull every clean library which has an upstream, including linked repos (`--no-links` to skip them) |
| `raft l link [SOURCE_DIR]` | Link library repos from `SOURCE_DIR` into `raftdevlibs` |
| `raft l unlink` | Remove links (all of them, or those given with `--libs`). The linked repos are not changed |
| `raft l remove --libs <LIB>...` (`raft l rm`) | Delete clones. Refused if a clone has uncommitted changes, stashes or commits not pushed to a remote, unless `--force` |

Every subcommand accepts `--dest <DEST_DIR>` to work on a folder other than `<app-folder>/raftdevlibs`, but the build only uses `raftdevlibs`. Use `raft l <subcommand> --help` for all the options.

### Fetching clones

```
raft l fetch [OPTIONS] [APPLICATION_FOLDER]
```

By default this fetches the libraries listed in `RAFT_COMPONENTS` (or `RaftCore`, `RaftSysMods`, `RaftI2C` and `RaftWebServer` if none are found) from the `robdobsn` GitHub account (`--account`) and checks out `main` (`--branch`). Use `--libs` to choose libraries. A library which is already in `raftdevlibs` is updated, but the command stops before updating a clone with uncommitted changes unless `--force` is given. Linked libraries are never changed by `fetch` - manage those repos directly.

### Linking repos

```
raft l link [SOURCE_DIR] [--libs <LIB>...] [--app <APPLICATION_FOLDER>] [--relative] [--force]
```

`SOURCE_DIR` is the folder containing the library repos, relative to the app folder (default `..`). For example, with the app in `~/dev/projects/MyApp` and the libraries in `~/dev/raft`:

```
raft l link ../../raft
raft l link ../../raft --libs RaftCore
```

Without `--libs` every library in `RAFT_COMPONENTS` found in `SOURCE_DIR` is linked. Links hold absolute paths unless `--relative` is given. An existing link to a different repo is only replaced with `--force`, and a clone is never replaced - delete it first with `raft l remove`.

On Windows, creating a symlink needs Developer Mode or Administrator rights, so `raft l link` makes a directory junction when symlinks aren't permitted. Junctions always hold absolute paths.

### Building with local libraries

- After a library is added to or removed from `raftdevlibs`, the next `raft build` deletes and recreates the SysType's build folder, because CMake doesn't notice the change by itself.
- Docker builds mount each linked repo into the container, as links to folders outside the project would otherwise not resolve there. A Docker build is refused if a linked repo would be mounted over a system folder in the container (for example a repo kept under `/opt`, where the ESP IDF image keeps its tools) or over the project itself; build with `--no-docker` or keep the repo somewhere else.
- A broken link (for example after its repo was moved) for a library the project uses stops the build with an error, so the build never quietly falls back to the remote version. A broken link for a library the project doesn't use only gives a warning.
- The `CMakeLists.txt` of apps created by RaftCLI 1.24.1 or older doesn't stop on a broken `raftdevlibs/RaftCore` link, so a build which bypasses `raft build` (for example `idf.py` run by hand in a container) quietly uses the published RaftCore. When `raftdevlibs/RaftCore` is a link `raft build` warns about this and, in a terminal, offers to add the check. `raft update-project` adds it too.
- On Windows, Docker builds of a project in a Windows folder are much slower than on Linux, because Docker Desktop shares every file with its Linux VM. Linking doesn't change this, but for a faster Docker build keep the project in the WSL filesystem and run `raft build` from WSL.

## Flash firmware to a development board (without rebuilding)

To flash firmware, use:

```
raft flash
OR
raft f
```

This will flash the firmare to the Espressif processor.

If the serial port is not specified (with the -p option) then the most likely suitable serial port will be tried. You can also specify baud rate for flashing using -f option. This program generally uses the espressif esptool to do the actual work of flashing. If you need to specify the full path to this tool then use the -t option.

By default, all partitions are flashed including the file system image. Use the --no-fs option to skip flashing the file system image - this is useful during iterative firmware development when the file system contents haven't changed. Use --fs to explicitly re-enable file system flashing if --no-fs was previously saved (see [Persistent Settings](#persistent-settings) below).

```
Flash firmware to the device

Usage: raft flash [OPTIONS] [APPLICATION_FOLDER]

Arguments:
  [APPLICATION_FOLDER]  Path to the application folder

Options:
  -s, --sys-type <SYS_TYPE>      System type to flash
  -p, --port <PORT>              Serial port
  -n, --native-serial-port       Native serial port when in WSL
  -f, --flash-baud <FLASH_BAUD>  Flash baud rate
  -t, --flash-tool <FLASH_TOOL>  Flash tool (e.g. esptool)
  -v, --vid <VID>                Vendor ID
      --no-fs                    Skip flashing the file system image
      --fs                       Flash the file system image (overrides saved --no-fs)
  -h, --help                     Print help
```

## Run esptool directly

You can run esptool directly through raftcli, which is useful for diagnostics and accessing esptool features directly:

```
raft esptool <esptool-arguments>
OR
raft e <esptool-arguments>
```

For example:
```
raft esptool version
raft esptool --port COM8 chip_id
raft esptool --port COM8 flash_id
```

This command automatically detects and uses esptool whether it's installed as a standalone executable or as a Python module (via `pip install esptool`). In WSL, it delegates to the Windows version unless the -n flag is specified.

```
Run esptool directly with arguments

Usage: raft esptool [OPTIONS] [ARGS]...

Arguments:
  [ARGS]...

Options:
  -n, --native-serial-port  Native serial port when in WSL
  -h, --help                Print help
```

## OTA (Over-the-air) Update Firmware (using WiFi/Ethernet connection)

To use OTA updates the device must be connected to a WiFi or Ethernet network and the IP address (or hostname) of the device must be known.

```
raft ota <IP_ADDRESS_OR_HOSTNAME>
OR
raft o <IP_ADDRESS_OR_HOSTNAME>
```

A connection (TCP) is made to the device and the standard HTTP POST protocol with form data is used to send the new firmware to the device. If required the -c option can be used which forces the use of the curl application to send the data - otherwise sending is done using the rust TcpStream mechanism which also permits rate and progress information to be shown.

Only the main binary of the application is written. The file-system and other partitions on the device including non-volatile storage are not affected by this operation.

```
Over-the-air update

Usage: raft ota [OPTIONS] <IP_ADDRESS_OR_HOSTNAME> [APPLICATION_FOLDER]

Arguments:
  <IP_ADDRESS_OR_HOSTNAME>  IP address or hostname for OTA
  [APPLICATION_FOLDER]      Path to the application folder

Options:
  -p, --ip-port <IP_PORT>    IP Port
  -s, --sys-type <SYS_TYPE>  System type to ota update
  -c, --use-curl             Use curl for OTA
  -h, --help                 Print help
```

## Managing files on a device

`raft fs` manages the files on a running device: list, download, upload, print, delete, mirror a folder, and the
file system's size. It works over any of the device's connections:

| Device | Connection |
| -- | -- |
| `192.168.86.52`, `MyDevice.local`, `http://host[:port]` | HTTP (the device's REST API) |
| `ws://host[:port][/path]` | The device's WebSocket (path default `/ws`), using RICREST |
| `serial:<port>[@baud]`, e.g. `serial:COM4`, `serial:/dev/ttyACM0` | The device's USB serial console, using RICREST alongside the log output. `serial:` on its own picks the port as `raft monitor` does |
| `ble:<name>` | BLE (see [BLE devices](#ble-devices)). `ble:` on its own uses the only Raft device in range |

The device is the first argument, or can be given with `-d/--device` or in the `RAFT_DEVICE` environment variable
(then leave it out of the arguments):

```
raft fs ls 192.168.86.52 -r                     # list every file
raft fs get serial:COM4 config.json             # download (to ./config.json; '-' for stdout)
raft fs put ws://MyDevice.local settings.json   # upload
RAFT_DEVICE=ble:Axiom018 raft fs cat config.json
```

| Command | Use |
| -- | -- |
| `raft fs list [FOLDER]` (`ls`) | List a folder (`-r` recursive, `--json` the device's raw reply) |
| `raft fs df` | Size, used and free space |
| `raft fs get <REMOTE> [LOCAL]` | Download a file (binary-safe on every connection) |
| `raft fs put <LOCAL> [REMOTE]` | Upload a file. The result is checked against the device's CRC of the file, and a damaged upload is deleted |
| `raft fs cat <REMOTE>` | Print a file |
| `raft fs delete <REMOTE>` (`rm`) | Delete a file (`-y` skips the confirmation) |
| `raft fs sync <LOCALDIR> [REMOTEDIR]` | Mirror a local folder onto the device: uploads new and changed files (`--hash` compares content, otherwise sizes), `--delete` removes files that aren't in the folder, `--dry-run` shows the plan |
| `raft fs format` | Reformat the file system (`-y` required; the device restarts) |
| `raft fs <DEVICE>` | Mirror the project's built FSImage (`build/<SysType>/raft/FSImage`) onto the device: changed files by content hash, and files not in the image deleted (`--no-delete`, `--no-hash`, `--exclude`, `--dry-run`, `-s <SysType>`) |

Every command takes `--fs sd` for a device's SD card (default `local`).

- Over serial the console port is shared with the device's log output: `--show-log` shows the log, and the port
  can't be open in `raft monitor` at the same time. `-v` shows link statistics at the end (frames dropped for a bad
  CRC, requests sent again).
- In WSL, serial and BLE devices are handed to the Windows `raft.exe` (as `raft flash` does), since WSL has no USB
  serial ports or Bluetooth; `-n` uses this system's ports instead.
- Rough speeds with an ESP32-S3 (they depend on the device's storage and firmware): HTTP and WebSocket upload at
  20-50 KB/s and download at 25-50 KB/s (the WebSocket is the faster for downloads); USB serial uploads at 10-16 KB/s
  and downloads at about 40 KB/s; BLE uploads at 15-25 KB/s and downloads at 5-8 KB/s.

## Device type records

`raft devtypes` (or `raft dt`) manages a project's device type records: the records compiled into the firmware and the records in a device type file loaded by a running device. Both are chosen in `systypes/Common/DeviceTypeSources.json` and `systypes/<SysType>/DeviceTypeSources.json` (see RaftCore's wiki page [Selecting Device Types](https://github.com/robdobsn/RaftCore/wiki/DeviceTypeSources)).

The selection rules are RaftCore's: the commands run `scripts/DeviceTypesTool.py` from the RaftCore the project builds with (`raftdevlibs/RaftCore`, or the build's copy in `build/<SysType>/_deps/raftcore-src`, or `--raftcore <dir>`), with Python 3.

```
raft devtypes list                          # records for the firmware and the type file, with their sources
raft devtypes init                          # create DeviceTypeSources.json selecting every RaftCore record
raft devtypes add raftcore:LSM6DS local:*   # add to the firmware's selection (--typefile: to the type file's)
raft devtypes remove raftcore:TrillRing     # remove (a record selected by a wildcard is excluded)
raft devtypes new MySensor --address 0x44   # a new record file in the project's records folder, to fill in
raft devtypes check                         # grammar, decode code and generator checks, without a build
raft devtypes update                        # take new commits of git sources (updates the lock file)
raft devtypes diff [KEY]                    # local copies and type file overrides against what they replace
raft devtypes typefile                      # build the device type file (build/<SysType>/raft/devlib.json)
raft devtypes push <DEVICE>                 # build it, check it on the device and apply it (the device restarts)
raft devtypes pull <DEVICE>                 # download the device's type file and compare it with the project's
raft devtypes status <DEVICE>               # what the device loaded, and whether it is the project's file
```

- `<DEVICE>` is the device's IP address or hostname (`http://host[:port]` also works), `ws://host[:port][/path]` for the device's WebSocket (path default `/ws`; faster than HTTP for downloads), `serial:<port>[@baud]` for a device on a USB serial port, e.g. `serial:COM4` or `serial:/dev/ttyACM0` (`serial:` on its own picks the port as `raft monitor` does), or `ble:<name>` for a device over BLE (see [BLE devices](#ble-devices)). It can be given with `-d/--device` instead, or in the `RAFT_DEVICE` environment variable; then leave it out of the arguments. The same applies to `raft fs`, e.g. `RAFT_DEVICE=serial:COM4 raft fs get config.json`.
- Over serial the device's console port is shared with its log output: `--show-log` shows the log, and `-v` shows link statistics at the end. The port can't be open in `raft monitor` at the same time. In WSL, serial devices are handed to the Windows `raft.exe` (as for `raft flash`; `-n` uses this system's ports instead).
- Name a device by hostname (e.g. `MyDevice.local`) rather than IP address when a command restarts it (`devtypes push`): it may come back with another address.
- `-s <SysType>` chooses the SysType (default: last built / first found); `init`, `add` and `remove` change the SysType's file, or Common's with `--common`.
- Type files are built against the records the last build compiled (`build/<SysType>/raft/DeviceTypeRecords.compiled.json`, which includes `DevTypes.json` and `DEV_TYPE_JSON_FILES` records), or the `builtin` selection if the project hasn't been built; `--compiled <file>` chooses another.
- `--config <file>` uses a `DeviceTypeSources.json` on its own instead of a project's, e.g. to make type files for devices running firmware built elsewhere.
- `push` uploads the file as `devlib.new.json`, has the device check it (the device's own rules decide, and its report is shown), then asks before applying it (`--yes` doesn't ask; `--dry-run` stops after the check and removes the upload). After the restart it confirms the device loaded the file. It warns if the device's file has changed since the last push from this project, and does nothing if the device already has the file (`--force` pushes anyway).
- Device commands work over every kind of device connection (see [Managing files on a device](#managing-files-on-a-device)); `-p` gives an HTTP port other than 80.

## Remote Debug Console

To connect to a device over WiFi (or Ethernet) for interactive debugging and log monitoring:

```
raft debug <IP_ADDRESS_OR_HOSTNAME>
OR
raft d <IP_ADDRESS_OR_HOSTNAME>
```

This opens a bidirectional TCP connection to the device's debug server. Log output from the device is displayed in real-time and commands can be typed and sent to the device. The connection auto-reconnects if it drops, retrying every 5 seconds.

The device must be running a TCP debug server (e.g. the Raft SerialConsole module) on the specified port.

As with the serial monitor, up-arrow and down-arrow cycle through command history, and logging to file can be enabled with the -l option.

To exit the remote debug console press ESC or Ctrl+C.

```
Start remote debug console

Usage: raft debug [OPTIONS] <IP_ADDRESS_OR_HOSTNAME> [APPLICATION_FOLDER]

Arguments:
  <IP_ADDRESS_OR_HOSTNAME>  Device address for debugging (hostname or IP)
  [APPLICATION_FOLDER]      Path to the application folder

Options:
  -p, --port <PORT>              Port for debugging [default: 8080]
  -l, --log                      Log debug console data to file
  -g, --log-folder <LOG_FOLDER>  Folder for log files [default: ./logs]
  -h, --help                     Print help
```

## Persistent Settings

Several command-line settings are automatically saved to `build/raft.info` when explicitly specified and reused on subsequent runs. This means you only need to specify options like the serial port or flash baud rate once - they will be remembered for future invocations.

| Setting | Saved by | Example |
| -- | -- | -- |
| `-p` serial port | `flash`, `run`, `monitor` | `raft flash -p COM8` |
| `-b` monitor baud rate | `run`, `monitor` | `raft monitor -b 921600` |
| `-f` flash baud rate | `flash`, `run` | `raft flash -f 2000000` |
| `-v` vendor ID | `flash`, `run`, `monitor` | `raft flash -v 0x10c4` |
| `--no-fs` / `--fs` | `flash`, `run` | `raft flash --no-fs` |

To override a saved setting, simply specify the option again on the command line. The new value will replace the previously saved one. For example, if `--no-fs` was previously used and saved, use `--fs` to re-enable file system flashing.

Build-related settings (system type, build method, ESP IDF path) are also saved automatically after a successful build. For each SysType the ESP IDF version and the libraries used from `raftdevlibs` are recorded too, so that the build folder is recreated when either changes.

### Build from source

If you want to build this app from source code then firstly [install rust](https://www.rust-lang.org/tools/install)

Clone the repo:

```
git clone https://github.com/robdobsn/RaftCLI
cd RaftCLI
cargo install --path . --locked
```

This will build and install the app into the binary executables folder that cargo uses. The `--locked` option uses the dependency versions recorded in `Cargo.lock` (without it cargo ignores that file and uses the newest compatible version of every dependency).

## Scaffolding Questions

The questions, in order, with their `--set` keys. Advanced ones are only asked with `--advanced` (otherwise their
defaults are used and shown in the summary). Sizes are measured on an ESP32-S3 with ESP-IDF 6.1 (RaftCLI
`devdocs/raft-new-questions-and-defaults-review.md`).

| Question | Key(s) | Explanation |
| -- | -- | -- |
| Project Name | `project_name` | name for your project |
| Hardware | `hardware`, `board`, `module`, `target_chip`, `flash_size`, `psram`, `psram_mode` | the chip, flash size, PSRAM and I2C pins - see [The hardware](#the-hardware) |
| App type | `app_type` | `general` (default: a connected device with a web interface) or `battery` (sleeps, wakes to measure and report). The type sets the defaults of the questions below, and with the connectivity answer some generated settings - see [App types](#app-types) |
| Wireless connectivity | `connectivity` | `wifi_ble` (default), `wifi` (about 240 KB smaller), `ble` (about 680 KB smaller) or `none` (USB serial only, about 970 KB smaller). Without WiFi, `add_compile_definitions(RAFT_NO_NETWORKING)` in `systypes/Common/features.cmake` leaves out the networking SysMods, WiFi driver and IP stack |
| BLE role | `ble_roles` | `peripheral` (apps and the raft CLI connect to the device), `central` (it receives BTHome sensor advertisements) or `both` - asked with BLE |
| Include the web server | `use_raft_webserver` | asked with WiFi |
| OTA updates | `ota` | `true`: two app slots, so the app can be updated over WiFi or BLE. `false`: one larger app slot (3 MB on 4 MB flash instead of 1.69 MB), updated only over USB. Not asked without WiFi or BLE (connectivity `none`), which always has one app slot |
| I2C and sensors | `i2c_sensors` | `common` (default: about 10 popular sensors), `extended` (about 25), `all` (all of RaftCore's), `i2c` (I2C with no sensors built in - add them later with a device type file) or `none` (no I2C). The sets are RaftCore's (`raftcore:@common`, `raftcore:@extended` in `systypes/Common/DeviceTypeSources.json`) |
| I2C SDA / SCL pin | `raft_i2c_sda_pin`, `raft_i2c_scl_pin` | asked unless a board preset gives them; the chip's usual pins are the default |
| Decoding of sensors added later | `runtime_decode` | how a device added later from a device type file is decoded on the device: `attrs` (from its attributes, the default), `code` (attributes and custom decode code, about 26 KB more) or `none` (only apps decode it, about 17 KB smaller) |
| Create User SysMod | `create_user_sysmod` | a SysMod for the main part of your application's code - SysMods allow user code to be managed like an Arduino app with setup() and loop() functions |
| User SysMod Class | `user_sys_mod_class` | the class name of that SysMod |
| System Type Name (advanced) | `sys_type_name` | the name of the main SysType (default: the project name) - SysTypes allow a project to target different hardware; additional SysTypes are added manually |
| User SysMod Name (advanced) | `user_sys_mod_name` | the SysMod's name (default: its class) |
| CPU core for the main task (advanced) | `main_task_core` | only for chips with more than one core (esp32, esp32s3, esp32p4). The main task runs the loop() function of every SysMod; WiFi, BLE and other system tasks run on core 0, so the default of 1 keeps the main loop from being held up by them (`CONFIG_ESP_MAIN_TASK_AFFINITY_CPU1=y` in the SysType's sdkconfig.defaults) |
| ESP-IDF Version (advanced) | `esp_idf_version` | written to systypes/Common/features.cmake as `set(ESP_IDF_VERSION ...)` - see [Which ESP IDF version is used](#which-esp-idf-version-is-used) |
| Use Raft SysMods (advanced) | `use_raft_sysmods` | RaftSysMods are building blocks (WiFi, BLE, file manager, serial console, ...) - almost every app uses them |

The partition table follows from the flash size and OTA answer: app slots as large as possible up to 3 MB, leaving
at least 512 KB for the file system. With WiFi, the device's access point gets a random password, saved in the app's
`SysTypes.json`.

The Raft libraries are fetched at `main` (the latest version); the tag can be changed later by editing, for example,
`RaftCore@<tag>` in `systypes/Common/features.cmake`. The tag of RaftCore also determines the `RaftBootstrap.cmake`
used to configure the build: a specific tag downloads that release's bootstrap (fully pinned build) while main uses
the latest release's bootstrap.

## Template App Details

New raft apps are "scaffolded" using template information in the raft_templates folder.

The handlebars templating library is used to fill in the gaps in the templates based on the answers to questions asked when running "raft new".

### The build bootstrap and older apps

The `CMakeLists.txt` of a raft app downloads `RaftBootstrap.cmake` from a RaftCore release. That script fetches RaftCore (and the other raft libraries) and then hands over to the rest of the build scripts inside the RaftCore it fetched. The `CMakeLists.txt` generated by `raft new` works out which release to download from the `RaftCore@<tag>` entry in `features.cmake` (the matching release for a tag, the latest release for `main`) so the two stay in step.

Apps created with older versions of RaftCLI have the URL of one specific release written into `CMakeLists.txt` (e.g. `.../releases/download/v1.37.1/RaftBootstrap.cmake`). If such an app uses a RaftCore which is not a fixed release (`RaftCore@main`, a branch, or no tag) then the bootstrap gets further and further behind the RaftCore it is used with, and `raft build` prints a warning. To fix it run `raft update-project` and do a clean build (`raft build -c`). `raft update-project` only replaces a `CMakeLists.txt` which hasn't been changed since it was generated; otherwise replace its bootstrap section by hand with the one from a newly generated app. An app which pins RaftCore to a release is assumed to be locked down deliberately and is not warned about.

`raft update-project [APPLICATION_FOLDER]` brings the `CMakeLists.txt` of an older app up to date. It adds the broken `raftdevlibs/RaftCore` link check (see [Building with local libraries](#building-with-local-libraries)) and replaces an unchanged old template as described above. It doesn't change anything it can't change safely.

The SysType's `sdkconfig.defaults` builds for size (`-Os`, assertions off) and, on the esp32 and esp32s3, runs the
CPU at 240 MHz (the ESP IDF default is 160 MHz). With BLE peripheral support the device advertises the RICREST BLE
service `aa76677e-9cfd-4626-a510-0d305be57c8d` (the RaftSysMods default, which raftjs looks for first; apps made
before October 2026 used `bb76677e-...`, which RaftCLI also finds).

In addition to generating source code, build files are generated for various build scenarios including:

* building natively on linux
* building using docker on mac, linux and windows

The best way to build a raft application depends on your OS and requirements. I primarily use a Windows machine with Ubuntu in WSL (google this if you are not familiar with running linux with the Windows Subsystem for Linux).  I find this is the best combination of speed and convenience and the Makefile generated by this app works well in that scenario when run from a WSL prompt.

## Appendix: Finding local Git changes

When using `raftdevlibs`, local Raft library checkouts can accumulate changes inside individual projects. The helper script `scripts/find_git_changes.py` scans a folder tree for Git repositories and prints only the repository folders that contain local changes. It doesn't follow links, so a library linked into `raftdevlibs` is reported once, at the repo's own location (if that is inside the folder scanned). For a single project, `raft l` shows the same information.

From a workspace folder containing multiple projects, run:

```sh
python3 raft/RaftCLI/scripts/find_git_changes.py .
```

The default output is intentionally one folder per line, for example:

```text
raft/RaftI2C/TestWebUI/raftdevlibs/RaftCore
raft/RaftI2C/TestWebUI/raftdevlibs/RaftWebServer
```

Useful options:

| Option | Use |
| -- | -- |
| `--absolute` | Print absolute paths instead of paths relative to the scan root |
| `--fail-on-changes` | Exit with status `1` when changed repositories are found |
| `--show-errors` | Show Git errors for folders that look like repositories but cannot be checked |

By default, Git errors from broken or generated repository markers are suppressed so the output remains a clean list of changed folders.