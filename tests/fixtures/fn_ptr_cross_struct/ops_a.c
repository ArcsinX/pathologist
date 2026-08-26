/* ops_a.c — Defines OpsA with "callback" at field index 0 */
#include "shared.h"

static int CallbackImplA(int x) { return x + 1; }

static struct OpsA g_opsA;
static int g_inited = 0;

void InitOpsA(void) {
    if (g_inited) return;
    g_inited = 1;
    g_opsA.callback = CallbackImplA;
    g_opsA.data = 42;
}

void RegisterOpsA(void **out) {
    InitOpsA();
    *out = &g_opsA;
}
