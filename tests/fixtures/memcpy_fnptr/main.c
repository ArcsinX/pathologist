#include <string.h>

void real_target(void) {
}

void ghost(void) {
}

/* Copies fn-ptr bits with memcpy instead of assignment — classic obfuscation. */
void memcpy_indirect(void) {
    void (*fp)(void);
    void (*src)(void) = &real_target;
    memcpy(&fp, &src, sizeof(fp));
    fp();
}

/* Must not infer ghost from unrelated memcpy of non-pointer data. */
void memcpy_no_fn_edge(void) {
    void (*fp)(void) = &real_target;
    char blob[32];
    memset(blob, 0, sizeof(blob));
    memcpy(blob, blob, 16);
    fp();
}
