/////////////////////////////////////////////////////////////////////////////////////////////////////////////////
//
// Main entry point
//
/////////////////////////////////////////////////////////////////////////////////////////////////////////////////

#include "RaftCoreApp.h"
{{{include_raft_sysmods}}}
{{{include_raft_webserver}}}
{{{include_user_sysmod}}}
{{{include_raft_i2c}}}

// Create the app
RaftCoreApp raftCoreApp;

// Entry point
extern "C" void app_main(void)
{
    {{{register_raft_sysmods}}}{{{register_raft_webserver}}}{{{register_raft_i2c}}}{{{register_user_sysmod}}}
    // Loop forever
    while (1)
    {
        // Loop the app
        raftCoreApp.loop();
    }
}
