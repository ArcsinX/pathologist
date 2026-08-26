#ifndef HDF_SBUF_H
#define HDF_SBUF_H

#include <stdint.h>
#include <stddef.h>
#include <stdbool.h>

struct HdfSBuf;
struct HdfSBufImpl;

enum HdfSbufType {
    SBUF_RAW = 0,
    SBUF_IPC,
    SBUF_TYPE_MAX,
};

struct HdfSBuf *HdfSbufTypedObtainCapacity(uint32_t type, size_t capacity);
struct HdfSBuf *HdfSbufTypedObtain(uint32_t type);
bool HdfSbufReadBuffer(struct HdfSBuf *sbuf, const void **data, uint32_t *readSize);
void HdfSbufRecycle(struct HdfSBuf *sbuf);
struct HdfSBufImpl *HdfSbufGetImpl(struct HdfSBuf *sbuf);

#endif
