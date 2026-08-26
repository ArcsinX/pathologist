#include "ops.hpp"
#include <stdint.h>

extern struct HdfSBuf *SbufObtainIpc(void);
extern struct ServiceOps *GetServiceOps(void);

void test_ipc_read(void) {
    struct HdfSBuf *sbuf = SbufObtainIpc();
    const void *data = nullptr;
    uint32_t size = 0;
    sbuf->impl->readBuffer(sbuf->impl, &data, &size);
}

void test_ipc_dispatch(void) {
    struct ServiceOps *ops = GetServiceOps();
    ops->Dispatch(nullptr, 1, nullptr, nullptr);
}

void test_service_dispatch(void *service) {
    struct ServiceOps *ops = GetServiceOps();
    ops->Dispatch(service, 2, nullptr, nullptr);
}
