#include "hdf_sbuf_impl.h"
#include <cstdlib>
#include <cstring>

struct MessageParcel {
    bool WriteUint32(uint32_t value) { (void)value; return true; }
    bool WriteUnpadBuffer(const void *data, uint32_t size) { (void)data; (void)size; return true; }
    uint32_t ReadUint32() { return 0; }
    const uint8_t *ReadUnpadBuffer(uint32_t size) { (void)size; return nullptr; }
};

static void MParcelImplInterfaceAssign(struct HdfSBufImpl *inf);

struct SBufMParcelImpl {
    explicit SBufMParcelImpl(MessageParcel *parcel = nullptr, bool owned = true)
        : realParcel(parcel), owned_(owned)
    {
        MParcelImplInterfaceAssign(&infImpl);
    }
    ~SBufMParcelImpl()
    {
        if (owned_ && realParcel != nullptr) {
            delete realParcel;
            realParcel = nullptr;
        }
    }
    struct HdfSBufImpl infImpl;
    MessageParcel *realParcel;
    bool owned_;
};

static MessageParcel *MParcelCast(struct HdfSBufImpl *impl)
{
    SBufMParcelImpl *sbufImpl = reinterpret_cast<SBufMParcelImpl *>(impl);
    return sbufImpl->realParcel;
}

static bool SbufMParcelImplWriteBuffer(struct HdfSBufImpl *impl, const uint8_t *data, uint32_t writeSize)
{
    auto parcel = MParcelCast(impl);
    return parcel->WriteUint32(writeSize) && parcel->WriteUnpadBuffer(data, writeSize);
}

static bool SbufMParcelImplWriteUint32(struct HdfSBufImpl *impl, uint32_t value)
{
    return MParcelCast(impl)->WriteUint32(value);
}

static bool SbufMParcelImplReadBuffer(struct HdfSBufImpl *impl, const uint8_t **data, uint32_t *readSize)
{
    if (data == nullptr || readSize == nullptr) {
        return false;
    }
    MessageParcel *parcel = MParcelCast(impl);
    *readSize = parcel->ReadUint32();
    *data = parcel->ReadUnpadBuffer(*readSize);
    return *data != nullptr;
}

static bool SbufMParcelImplReadUint32(struct HdfSBufImpl *impl, uint32_t *value)
{
    if (value == nullptr) {
        return false;
    }
    *value = MParcelCast(impl)->ReadUint32();
    return true;
}

static void SbufMParcelImplRecycle(struct HdfSBufImpl *impl)
{
    SBufMParcelImpl *sbufImpl = reinterpret_cast<SBufMParcelImpl *>(impl);
    delete sbufImpl;
}

static void MParcelImplInterfaceAssign(struct HdfSBufImpl *inf)
{
    inf->writeBuffer = SbufMParcelImplWriteBuffer;
    inf->writeUint32 = SbufMParcelImplWriteUint32;
    inf->readBuffer = SbufMParcelImplReadBuffer;
    inf->readUint32 = SbufMParcelImplReadUint32;
    inf->recycle = SbufMParcelImplRecycle;
}

extern "C" struct HdfSBufImpl *SbufObtainIpc(size_t capacity)
{
    (void)capacity;
    struct SBufMParcelImpl *sbuf = new SBufMParcelImpl(new MessageParcel());
    return &sbuf->infImpl;
}
