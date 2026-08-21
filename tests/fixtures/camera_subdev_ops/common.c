#include "ops.h"

static void fill_subdev(struct SubDevice *subDev)
{
    subDev->subDevOps = GetSensorDeviceOps();
}

void CommonDeviceSetConfig(void)
{
    struct SubDevice subDev;
    fill_subdev(&subDev);
    subDev.subDevOps->setConfig(subDev);
}
