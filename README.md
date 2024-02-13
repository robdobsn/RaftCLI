# Raft command line interface

Raft is an opinionated operating environment for the Espressif ESP32 family.

This command-line application is used to scaffold raft apps and provide serial monitoring for them.

## Usage

###Creating a new raft application:

```
$> raft new .
```
This creates a new raft app in the current folder - change . to your choice of folder to create the app elsewhere.

Additonal option:
-c clean (delete) the contents of the folder before creating the app

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

### Serial monitor

```
raft monitor <port>
```
Where <port> is the serial port to be monitored
Additional options:
-b set the baud rate
-l log to a log file (files are automatically named based on date and time)
-g set the name of the folder to log into (defaults to ./logs)