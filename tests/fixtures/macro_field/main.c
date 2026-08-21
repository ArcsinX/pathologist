struct Inner {
    int *p;
};

struct Outer {
    struct Inner inner;
};

#define FIELD_P(o) ((o)->inner.p)

void sink(int *x);

void macro_assign(struct Outer *o, int *v) {
    FIELD_P(o) = v;
}

void macro_user(void) {
    struct Outer o;
    int local;
    macro_assign(&o, &local);
    sink(o.inner.p);
}
