struct L3 { int *p; };
struct L2 { struct L3 c; };
struct L1 { struct L2 b; };

void sink(int *x);

void set_deep(struct L1 *o, int *v) {
    o->b.c.p = v;
}

void user(void) {
    struct L1 root;
    int local;
    set_deep(&root, &local);
    sink(root.b.c.p);
}
