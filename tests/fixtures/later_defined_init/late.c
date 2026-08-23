/* Regression fixture: functions referenced BEFORE their definition, with no
 * forward declaration. Lowering must not depend on encounter order (see
 * PendingFnRef deferred resolution in lower.rs). */

struct Ops {
    int (*init)(int);
};

static struct Ops g_tbl;

static int Caller(int v)
{
    return g_tbl.init(v);
}

void Bind(void)
{
    g_tbl.init = LaterBody;
}

static int LaterBody(int v)
{
    return v + 7;
}

static int (*g_fp)(int);

void BindFp(void)
{
    g_fp = LaterInit;
}

static int LaterInit(int v)
{
    return v + 9;
}

static int UseFp(int v)
{
    return g_fp(v);
}
