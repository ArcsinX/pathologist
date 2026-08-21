void touch(int *p);

void anon_user(void) {
    struct { int tag; int *payload; } row;
    int value;
    row.payload = &value;
    touch(row.payload);
}
