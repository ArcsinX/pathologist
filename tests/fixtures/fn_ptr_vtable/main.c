struct Sub {
    void (*handler)(int *);
};

struct Ops {
    struct Sub *interFace;
};

static void target(int *p) {
    (void)p;
}

static struct Sub g_sub = {
    .handler = target,
};

static struct Ops g_ops;

void dispatch(int *v) {
    g_ops.interFace = &g_sub;
    g_ops.interFace->handler(v);
}
