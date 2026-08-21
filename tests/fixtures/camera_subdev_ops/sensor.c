#include "ops.h"

static struct DeviceConfigOps g_sensorDeviceOps = {
    .setConfig = &CameraCmdSensorSetConfig,
};

struct DeviceConfigOps *GetSensorDeviceOps(void)
{
    return &g_sensorDeviceOps;
}

int CameraCmdSensorSetConfig(struct SubDevice subDev)
{
    (void)subDev;
    return 0;
}
