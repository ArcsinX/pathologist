void a(void) { }
void b(void) { }
void c(void) { }

void narrowed(void) {
    void (*fp)(void) = &a;
    fp();
}

void ambiguous(void) {
    void (*fp)(void);
    fp = &a;
    fp();
}
