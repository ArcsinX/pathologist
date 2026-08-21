void row0(void) {
}

void row1(void) {
}

void dispatch_table(void) {
    void (*table[2])(void) = {row0, row1};
    table[0]();
}
