#define INVOKE(f) ((f)())

void target(void) {
}

void decoy(void) {
}

void via_macro_indirect(void) {
    void (*fp)(void) = &target;
    INVOKE(fp);
}

void via_macro_direct_name(void) {
    INVOKE(decoy);
}
