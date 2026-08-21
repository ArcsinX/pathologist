static int *slot;

static int *GetOps(void) {
    return slot;
}

void user(int **out) {
    *out = GetOps();
}
