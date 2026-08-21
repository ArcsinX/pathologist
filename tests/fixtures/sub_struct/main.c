struct Inner {
    int *p;
};

struct Outer {
    struct Inner inner;
};

void sink(int *x);

void assign_field(struct Outer *o, int *v) {
    o->inner.p = v;
}

void user(void) {
    struct Outer o;
    int local;
    assign_field(&o, &local);
    sink(o.inner.p);
}
