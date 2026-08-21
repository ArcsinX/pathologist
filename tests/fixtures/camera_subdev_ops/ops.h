struct SubDevice {
    int x;
    struct DeviceConfigOps *subDevOps;
};

struct DeviceConfigOps {
    int (*setConfig)(struct SubDevice);
};

int CameraCmdSensorSetConfig(struct SubDevice subDev);
struct DeviceConfigOps *GetSensorDeviceOps(void);
