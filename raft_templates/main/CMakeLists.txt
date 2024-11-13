# Register main component
idf_component_register(
    SRCS
        "main.cpp"
    INCLUDE_DIRS
        "."
    REQUIRES
        RaftCore{{{depends_raft_sysmods}}}{{{depends_raft_webserver}}}{{{depends_raft_i2c}}}{{{depends_user_sysmod}}}
)
