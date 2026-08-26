/* ops_b.c — Defines OpsB with "handler" at field index 0 (same position) */
#include "shared.h"

static int HandlerImplB(int x) { return x + 100; }

static struct OpsB g_opsB;
static int g_inited = 0;

void InitOpsB(void) {
    if (g_inited) return;
    g_inited = 1;
    g_opsB.handler = HandlerImplB;
    g_opsB.count = 10;
}

void RegisterOpsB(void **out) {
    InitOpsB();
    *out = &g_opsB;
}
