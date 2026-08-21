void target(int *p);
void dispatcher(void (*fp)(int *), int *v);

void use_fn_ptr(void (*cb)(int *), int *data) {
    cb(data);
}

void run(void) {
    int x = 0;
    void (*fp)(int *) = &target;
    fp(&x);
}
