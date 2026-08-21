void path_a(void) {
}

void path_b(void) {
}

void ambiguous_branch(int flag) {
    void (*fp)(void);
    if (flag) {
        fp = &path_a;
    } else {
        fp = &path_b;
    }
    fp();
}
