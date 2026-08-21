void alpha(void) {
}

void beta(void) {
}

void comma_indirect(void) {
    void (*fp)(void);
    (void)(fp = &alpha), fp();
}

void comma_still_alpha(void) {
    void (*fp)(void) = &alpha;
    (void)0, fp();
}
