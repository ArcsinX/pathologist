#include "ops.h"

void target(int *p) {
    (void)p;
}

static struct Ops g_ops = {
    .handler = target,
};

void caller(int *v) {
    g_ops.handler(v);
}
