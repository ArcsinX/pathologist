#include <string.h>

void embedded(void) {
}

void struct_holder_memcpy(void) {
    struct Holder {
        void (*fp)(void);
    } h;

    struct Holder src;
    src.fp = &embedded;
    memcpy(&h, &src, sizeof(h));
    h.fp();
}
