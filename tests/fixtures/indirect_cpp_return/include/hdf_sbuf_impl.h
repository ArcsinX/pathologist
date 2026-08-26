#ifndef HDF_SBUF_IMPL_H
#define HDF_SBUF_IMPL_H

#include <stdint.h>
#include <stddef.h>
#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

struct HdfSbufConstructor {
    struct HdfSBufImpl *(*obtain)(size_t capacity);
    struct HdfSBufImpl *(*bind)(uintptr_t base, size_t size);
};

struct HdfSBufImpl {
    bool (*writeBuffer)(struct HdfSBufImpl *sbuf, const uint8_t *data, uint32_t writeSize);
    bool (*writeUint32)(struct HdfSBufImpl *sbuf, uint32_t value);
    bool (*readBuffer)(struct HdfSBufImpl *sbuf, const uint8_t **data, uint32_t *readSize);
    bool (*readUint32)(struct HdfSBufImpl *sbuf, uint32_t *value);
    void (*recycle)(struct HdfSBufImpl *sbuf);
};

#ifdef __cplusplus
}
#endif

#endif
