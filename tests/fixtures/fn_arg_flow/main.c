void handler(void) {}

void register_cb(void (*cb)(void)) {
    (void)cb;
}

void user(void) {
    register_cb(handler);
}
