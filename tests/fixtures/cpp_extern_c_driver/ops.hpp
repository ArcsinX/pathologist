#ifndef OPS_HPP
#define OPS_HPP

#include <stdint.h>

struct HdfSBufImpl {
    void *buf;
    uint32_t size;
    uint32_t dataOwn;
    void (*readBuffer)(void*, const void**, uint32_t*);
    void (*readUint32)(void*, uint32_t*);
    void (*writeBuffer)(void*, const void*, uint32_t);
    void (*writeUint32)(void*, uint32_t);
};

struct HdfSBuf {
    struct HdfSBufImpl *impl;
};

struct ServiceOps {
    int32_t (*Open)(void *service, int32_t fd);
    int32_t (*Dispatch)(void *service, int32_t code, void *data, void *reply);
    void (*Close)(void *service);
};

#endif
