struct Ops {
    void (*handler)(int *);
};

static struct Ops g_ops;

static void target(int *p) {
    (void)p;
}

void init(void) {
    g_ops.handler = target;
}

void caller(int *v) {
    init();
    g_ops.handler(v);
}
