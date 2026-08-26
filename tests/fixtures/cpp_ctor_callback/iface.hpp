#ifndef CTOR_CB_HPP
#define CTOR_CB_HPP

#include <stdint.h>

struct IDeviceIoService {
    int32_t (*Open)(void *service);
    int32_t (*Dispatch)(void *service, int32_t code, void *data, void *reply);
    void (*Release)(void *service);
};

struct HdfDeviceObject {
    struct IDeviceIoService *service;
    void *device;
};

struct DriverEntry {
    int32_t (*Bind)(struct HdfDeviceObject *device);
    int32_t (*Init)(struct HdfDeviceObject *device);
    void (*Release)(struct HdfDeviceObject *device);
};

typedef int32_t (*Constructor)(void);

#endif
