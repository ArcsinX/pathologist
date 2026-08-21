void through_cast(void) {
}

void cast_indirect(void) {
    void (*fp)(void) = &through_cast;
    void *opaque = (void *)fp;
    void (*again)(void) = (void (*)(void))opaque;
    again();
}
