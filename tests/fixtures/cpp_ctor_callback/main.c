#include "iface.hpp"
#include <stdint.h>

extern bool RegisterConstructor(const char *name, Constructor ctor);
extern void *CreateService(const char *name);

static int32_t MyConstructor(void) {
    return 0;
}

void test_callback_dispatch(void) {
    RegisterConstructor("my_svc", MyConstructor);
    void *svc = CreateService("my_svc");
    if (svc) {
        struct HdfDeviceObject dev = {0};
        struct DriverEntry entry = {
            .Bind = nullptr,
            .Init = nullptr,
            .Release = nullptr,
        };
        entry.Init(&dev);
        if (dev.service) {
            dev.service->Dispatch(svc, 1, nullptr, nullptr);
        }
    }
}
