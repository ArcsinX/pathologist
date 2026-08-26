#include "hdf_sbuf.h"
#include "hdf_sbuf_impl.h"
#include <stdlib.h>
#include <string.h>

struct HdfSBuf {
    struct HdfSBufImpl *impl;
    uint32_t type;
};

struct HdfSBufImpl *SbufObtainRaw(size_t capacity);
struct HdfSBufImpl *SbufObtainIpc(size_t capacity) __attribute__((weak));

static struct HdfSbufConstructor g_constructorMap[SBUF_TYPE_MAX] = {
    [SBUF_RAW] = {
        .obtain = SbufObtainRaw,
    },
    [SBUF_IPC] = {
        .obtain = SbufObtainIpc,
    },
};

struct HdfSBuf *HdfSbufTypedObtainCapacity(uint32_t type, size_t capacity)
{
    struct HdfSBuf *sbuf = NULL;
    const struct HdfSbufConstructor *constructor = &g_constructorMap[type];
    if (constructor->obtain == NULL) {
        return NULL;
    }
    sbuf = (struct HdfSBuf *)malloc(sizeof(struct HdfSBuf));
    if (sbuf == NULL) {
        return NULL;
    }
    sbuf->impl = constructor->obtain(capacity);
    if (sbuf->impl == NULL) {
        free(sbuf);
        return NULL;
    }
    sbuf->type = type;
    return sbuf;
}

struct HdfSBuf *HdfSbufTypedObtain(uint32_t type)
{
    return HdfSbufTypedObtainCapacity(type, 256);
}

bool HdfSbufReadBuffer(struct HdfSBuf *sbuf, const void **data, uint32_t *readSize)
{
    if (sbuf == NULL || sbuf->impl == NULL || sbuf->impl->readBuffer == NULL) {
        return false;
    }
    return sbuf->impl->readBuffer(sbuf->impl, (const uint8_t **)data, readSize);
}

void HdfSbufRecycle(struct HdfSBuf *sbuf)
{
    if (sbuf == NULL) {
        return;
    }
    if (sbuf->impl != NULL && sbuf->impl->recycle != NULL) {
        sbuf->impl->recycle(sbuf->impl);
    }
    free(sbuf);
}

struct HdfSBufImpl *HdfSbufGetImpl(struct HdfSBuf *sbuf)
{
    if (sbuf == NULL) {
        return NULL;
    }
    return sbuf->impl;
}
