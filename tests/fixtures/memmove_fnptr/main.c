#include <string.h>

void mover_target(void) {
}

void memmove_indirect(void) {
    void (*fp)(void);
    void (*src)(void) = &mover_target;
    char scratch[32];
    memmove(scratch, &src, sizeof(src));
    memmove(&fp, scratch, sizeof(fp));
    fp();
}
