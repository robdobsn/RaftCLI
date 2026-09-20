# Set the target Espressif chip
set(IDF_TARGET "{{target_chip}}")

# ESP-IDF version used to build (for both local and Docker builds using "raft build")
# This is the default for all SysTypes - it can be overridden in systypes/<SysType>/features.cmake
# It must be a literal version on one line as it is also read by the raft command line tool
set(ESP_IDF_VERSION "{{esp_idf_version}}")

# Raft components
set(RAFT_COMPONENTS
    RaftCore@{{raft_core_git_tag}}
    {{inc_raft_sysmods}}
    {{inc_raft_webserver}}
    {{{inc_raft_i2c_sysmod}}}
)

# Console logging timeout (only applies when the console is on the USB Serial/JTAG peripheral)
# When the USB transmit buffer is full (e.g. no USB host is connected or the host isn't reading) a
# log write can block the calling task for up to this time - which can include the main loop task.
# A small value means log output is dropped instead of stalling the task. The RaftCore default is 100ms.
add_compile_definitions(RAFT_LOGGER_USB_JTAG_WRITE_TIMEOUT_MS=10)

# Main task checks (debug)
# Most of the Raft framework (SysMods, comms channels, web server connections, config, etc) is owned by
# the main task - the task which runs loop() for every SysMod - and is not protected by locks.
# If a main-task-only function is called from another task then by default an error is logged
# ("... called from task other than main - NOT THREAD SAFE").
# Uncomment the following line to abort() instead so that the offending call can be found from the backtrace
# add_compile_definitions(RAFT_MAIN_TASK_CHECK_ABORT)

# File system
set(FS_TYPE "littlefs")
set(FS_IMAGE_PATH "../Common/FSImage")

# Web UI

# Uncomment the "set" line below if you want to use the web UI
# This assumes an app is built using npm run build
# it also assumes that the web app is built into a folder called "dist" in the UI_SOURCE_PATH
# set(UI_SOURCE_PATH "../Common/WebUI")

# Uncomment the following line if you do NOT want to gzip the web UI
# set(WEB_UI_GEN_FLAGS ${WEB_UI_GEN_FLAGS} --nogzip)

# Uncomment the following line to include a source map for the web UI - this will increase the size of the web UI
# set(WEB_UI_GEN_FLAGS ${WEB_UI_GEN_FLAGS} --incmap)
