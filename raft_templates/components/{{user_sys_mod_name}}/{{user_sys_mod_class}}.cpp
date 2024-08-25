////////////////////////////////////////////////////////////////////////////////
//
// {{user_sys_mod_class}}.cpp
//
////////////////////////////////////////////////////////////////////////////////

#include "{{user_sys_mod_class}}.h"
#include "RaftUtils.h"

{{user_sys_mod_class}}::{{user_sys_mod_class}}(const char *pModuleName, RaftJsonIF& sysConfig)
    : RaftSysMod(pModuleName, sysConfig)
{
    // This code is executed when the system module is created
    // ...
}

{{user_sys_mod_class}}::~{{user_sys_mod_class}}()
{
    // This code is executed when the system module is destroyed
    // ...
}

void {{user_sys_mod_class}}::setup()
{
    // The following code is an example of how to use the config object to
    // get a parameter from SysType (JSON) file for this system module
    // Replace this with your own setup code
    String configValue = config.getString("exampleGroup/exampleKey", "This Should Not Happen!");
    LOG_I(MODULE_PREFIX, "%s", configValue.c_str());
}

void {{user_sys_mod_class}}::loop()
{
    // Check for loop rate
    if (Raft::isTimeout(millis(), _lastLoopMs, 1000))
    {
        // Update last loop time
        _lastLoopMs = millis();

        // Put some code here that will be executed once per second
        // ...
    }
}

