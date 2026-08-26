#include "hdf_sbuf.h"
#include "hdf_sbuf_impl.h"
#include <stdlib.h>
#include <string.h>

struct HdfSBufRaw {
    struct HdfSBufImpl infImpl;
    uint8_t *data;
    size_t capacity;
};

static bool SbufRawImplReadBuffer(struct HdfSBufImpl *impl, const uint8_t **data, uint32_t *readSize)
{
    (void)impl;
    (void)data;
    (void)readSize;
    return true;
}

static bool SbufRawImplWriteBuffer(struct HdfSBufImpl *impl, const uint8_t *data, uint32_t writeSize)
{
    (void)impl;
    (void)data;
    (void)writeSize;
    return true;
}

static bool SbufRawImplReadUint32(struct HdfSBufImpl *impl, uint32_t *value)
{
    (void)impl;
    (void)value;
    return true;
}

static bool SbufRawImplWriteUint32(struct HdfSBufImpl *impl, uint32_t value)
{
    (void)impl;
    (void)value;
    return true;
}

static void SbufRawImplRecycle(struct HdfSBufImpl *impl)
{
    struct HdfSBufRaw *sbuf = (struct HdfSBufRaw *)impl;
    if (sbuf != NULL) {
        if (sbuf->data != NULL) {
            free(sbuf->data);
        }
        free(sbuf);
    }
}

static void SbufInterfaceAssign(struct HdfSBufImpl *inf)
{
    inf->writeBuffer = SbufRawImplWriteBuffer;
    inf->writeUint32 = SbufRawImplWriteUint32;
    inf->readBuffer = SbufRawImplReadBuffer;
    inf->readUint32 = SbufRawImplReadUint32;
    inf->recycle = SbufRawImplRecycle;
}

struct HdfSBufImpl *SbufObtainRaw(size_t capacity)
{
    struct HdfSBufRaw *sbuf = (struct HdfSBufRaw *)calloc(1, sizeof(struct HdfSBufRaw));
    if (sbuf == NULL) {
        return NULL;
    }
    sbuf->data = (uint8_t *)calloc(1, capacity);
    if (sbuf->data == NULL) {
        free(sbuf);
        return NULL;
    }
    sbuf->capacity = capacity;
    SbufInterfaceAssign(&sbuf->infImpl);
    return &sbuf->infImpl;
}
