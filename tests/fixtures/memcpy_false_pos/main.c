#include <string.h>

void fn_a(void) {
}

void fn_b(void) {
}

/*
 * Store fn_a into fp, memcpy its bytes into an unrelated buffer.
 * Indirect call must stay on fn_a only (must-analysis: no spurious fn_b).
 */
void memcpy_side_buffer(void) {
    void (*fp)(void) = &fn_a;
    void (*other)(void) = &fn_b;
    char side[16];
    (void)other;
    memcpy(side, &fp, sizeof(fp));
    fp();
}
