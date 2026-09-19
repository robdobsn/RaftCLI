////////////////////////////////////////////////////////////////////////////////
//
// {{user_sys_mod_class}}.cpp
//
////////////////////////////////////////////////////////////////////////////////

#include "{{user_sys_mod_class}}.h"
#include "RaftUtils.h"

// Which task runs what (important if you create your own tasks or use callbacks)
//
// - setup(), loop() and REST API endpoint handlers all run on the main task - the task which calls loop()
//   on every SysMod in turn. Code in these functions can use the Raft framework freely but shouldn't block
//   for long as that stalls every other SysMod (a warning is logged if a SysMod's loop() is slow - see
//   slowSysModMs in SysTypes.json).
//
// - Most of the framework is owned by the main task and is NOT protected by locks - including publishing
//   and sending messages (e.g. on a websocket), the config object, network/WiFi control, device manager
//   registration and LED patterns. Don't call these from another task. If you do then the error
//   "... called from task other than main - NOT THREAD SAFE" is logged.
//
// - Some callbacks do NOT run on the main task. In particular device data-change callbacks for devices on a
//   bus (registered with registerForDeviceData) and bus poll-result callbacks run on the bus's worker task
//   (e.g. the I2C task) which may be running on the other core at the same time as loop().
//   Device status-change callbacks (registerForDeviceStatusChange) do run on the main task.
//
// - To get data from a callback (or from your own task) to the main task hand it over using a
//   ThreadSafeQueue (see ThreadSafeQueue.h) or a std::atomic value and then act on it in loop().
//   Keep callbacks short and never block in them.

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

