# Raft command line interface

Raft is an opinionated operating environment for the Espressif ESP32 family - see [further information on Raft](https://github.com/robdobsn/RaftCore)

This command-line application is used to scaffold raft apps and provide serial monitoring for them.

## Usage


### Creating a new raft application:

```
$ raft new .
```
This creates a new raft app in the current folder. Change . to your choice of folder to create the app elsewhere.

You will be asked a series of questions including the name of your app, target chip, etc.


After execution you will have a folder structure like this:

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

### Monitoring a Serial port

```
raft monitor <port>
```
Where <port> is the serial port to be monitored

Additional options:

```
Monitor a serial port

Usage: raft.exe monitor [OPTIONS] [PORT]

Arguments:
  [PORT]

Options:
  -b, --baud <BAUD>              Baud rate
  -l, --log                      Log serial data to file
  -g, --log-folder <LOG_FOLDER>  Folder for log files [default: ./logs]
  -h, --help                     Print help
```

## Installation

There are several ways to install this app as it depends somewhat on the operating system you are using.

* Maybe you already have the rust language installed or are happy to install it - since you are here I assume you are interested in embedded development rust may be in your future in any case :) If so follow the "I like Rust" method below
* Or maybe you prefer download a binary and put it on your OS path so you can run it on your machine

### I like Rust

Follow the steps to [install rust](https://www.rust-lang.org/tools/install)

Then install the app directly with:

```
cargo install raftcli
```

### Install a pre-build binary

Select a binary executable from the [releases folder](https://github.com/robdobsn/RaftCLI/releases)

Download the compressed archive and uncompress. Then copy the binary executable to a folder which is on the PATH of your operating system.

### Build from source

If you want to build this app from source code then firstly [install rust](https://www.rust-lang.org/tools/install)

Clone the repo:

```
git clone https://github.com/robdobsn/RaftCLI
cd RaftCLI
cargo install --path .
```

This will build and install the app into the binary executables folder that cargo uses.
