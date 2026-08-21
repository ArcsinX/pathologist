void target(int *p) {
    (void)p;
}

void caller(void) {
    int x;
    void (*fp)(int *) = &target;
    fp(&x);
}
