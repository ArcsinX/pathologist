#include "iface.hpp"
#include <map>
#include <string>

static std::map<std::string, Constructor> g_constructorMap;

bool RegisterConstructor(const char *name, Constructor ctor) {
    g_constructorMap[std::string(name)] = ctor;
    return true;
}

void *CreateService(const char *name) {
    auto it = g_constructorMap.find(std::string(name));
    if (it != g_constructorMap.end()) {
        it->second();
        return (void*)1;
    }
    return nullptr;
}

static int32_t SampleDispatch(void *service, int32_t code, void *data, void *reply) {
    (void)service;
    (void)code;
    (void)data;
    (void)reply;
    return 0;
}

static struct IDeviceIoService g_sampleService = {
    .Open = nullptr,
    .Dispatch = SampleDispatch,
    .Release = nullptr,
};

extern "C" int32_t SampleDriverInit(struct HdfDeviceObject *device) {
    device->service = &g_sampleService;
    return 0;
}

extern "C" struct DriverEntry g_sampleDriverEntry = {
    .Bind = nullptr,
    .Init = SampleDriverInit,
    .Release = nullptr,
};
