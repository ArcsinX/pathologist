static void helper(int *p) {
    (void)p;
}

void caller(int *v) {
    helper(v);
}
