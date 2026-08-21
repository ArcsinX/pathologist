struct Inner {
    int *p;
};

struct Outer {
    struct Inner inner;
};

/* Function-like macro syntax — our preproc treats this as object macro + stray (o). */
#define FIELD_P(o) ((o)->inner.p)

void broken_macro_assign(struct Outer *o, int *v) {
    FIELD_P(o) = v;
}
