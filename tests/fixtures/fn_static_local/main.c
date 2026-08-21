static void target(void) {}

void user(void) {
    static void (*handler)(void);
    handler = target;
    handler();
}
