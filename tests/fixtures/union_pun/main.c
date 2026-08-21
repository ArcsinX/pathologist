union Data {
    int i;
    int *p;
};

void consume(int *q);

void union_user(void) {
    union Data u;
    int x;
    u.p = &x;
    consume(u.p);
}
