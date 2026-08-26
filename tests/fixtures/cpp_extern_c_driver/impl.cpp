#include "ops.hpp"
#include <new>

struct SBufMParcelImpl {
    struct HdfSBufImpl infImpl;
    int32_t fd;
};

static void MParcelReadBuffer(void *sbuf, const void **data, uint32_t *size) {
    SBufMParcelImpl *impl = reinterpret_cast<SBufMParcelImpl*>(sbuf);
    (void)impl;
    *data = nullptr;
    *size = 0;
}

static void MParcelReadUint32(void *sbuf, uint32_t *value) {
    SBufMParcelImpl *impl = reinterpret_cast<SBufMParcelImpl*>(sbuf);
    (void)impl;
    *value = 0;
}

static void MParcelWriteBuffer(void *sbuf, const void *data, uint32_t size) {
    SBufMParcelImpl *impl = reinterpret_cast<SBufMParcelImpl*>(sbuf);
    (void)impl;
    (void)data;
    (void)size;
}

static void MParcelWriteUint32(void *sbuf, uint32_t value) {
    SBufMParcelImpl *impl = reinterpret_cast<SBufMParcelImpl*>(sbuf);
    (void)impl;
    (void)value;
}

static void MParcelImplInterfaceAssign(struct HdfSBufImpl *inf) {
    inf->readBuffer = MParcelReadBuffer;
    inf->readUint32 = MParcelReadUint32;
    inf->writeBuffer = MParcelWriteBuffer;
    inf->writeUint32 = MParcelWriteUint32;
}

extern "C" struct HdfSBuf *SbufObtainIpc(void) {
    SBufMParcelImpl *impl = new SBufMParcelImpl();
    MParcelImplInterfaceAssign(&impl->infImpl);
    struct HdfSBuf *sbuf = new struct HdfSBuf();
    sbuf->impl = &impl->infImpl;
    return sbuf;
}

static int32_t ServiceDispatch(void *service, int32_t code, void *data, void *reply) {
    (void)service;
    (void)code;
    (void)data;
    (void)reply;
    return 0;
}

static struct ServiceOps g_ops = {
    .Open = nullptr,
    .Dispatch = ServiceDispatch,
    .Close = nullptr,
};

extern "C" struct ServiceOps *GetServiceOps(void) {
    return &g_ops;
}
