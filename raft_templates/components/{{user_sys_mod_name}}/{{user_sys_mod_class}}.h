////////////////////////////////////////////////////////////////////////////////
//
// {{user_sys_mod_class}}.h
//
////////////////////////////////////////////////////////////////////////////////

#pragma once

#include "RaftArduino.h"
#include "RaftSysMod.h"

class {{user_sys_mod_class}} : public RaftSysMod
{
public:
    {{user_sys_mod_class}}(const char *pModuleName, RaftJsonIF& sysConfig);
    virtual ~{{user_sys_mod_class}}();

    // Create function (for use by SysManager factory)
    static RaftSysMod* create(const char* pModuleName, RaftJsonIF& sysConfig)
    {
        return new {{user_sys_mod_class}}(pModuleName, sysConfig);
    }

protected:

    // Setup
    virtual void setup() override final;

    // Loop (called frequently)
    virtual void loop() override final;

private:
    // Example of how to control loop rate
    uint32_t _lastLoopMs = 0;
};
