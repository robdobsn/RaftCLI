# Set the target Espressif chip
set(IDF_TARGET "{{target_chip}}")

# Raft components
set(RAFT_COMPONENTS
    RaftCore@{{raft_core_git_tag}}
    {{inc_raft_sysmods}}
    {{inc_raft_webserver}}
    {{{inc_raft_i2c_sysmod}}}
)

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
