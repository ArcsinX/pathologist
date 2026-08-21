void target(int *p) {
    (void)p;
}

void dispatcher(void (*fp)(int *), int *v) {
    fp(v);
}
