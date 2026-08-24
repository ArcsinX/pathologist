/* Array-of-struct dispatch tables: designated initializers, tentative
 * definitions, and runtime stores into array-element fields must all
 * produce resolvable indirect-call targets. Mirrors the
 * g_sbufConstructorMap / g_tbl patterns seen in real driver code. */

struct Ctor {
    int (*obtain)(int);
    int (*bind)(int);
};

static int raw_obtain(int x) { return x + 1; }
static int ipc_obtain(int x) { return x + 2; }
static int hw_bind(int x) { return x + 3; }

/* 1. Global array, subscript+field designated initializers (fx5/fx7/fx15). */
static struct Ctor g_map[3] = {
    [0] = { .obtain = raw_obtain },
    [1] = { .obtain = ipc_obtain },
    [2] = { .bind = hw_bind },
};

static struct Ctor *map_get(int type)
{
    if (type < 0 || type > 2)
        return 0;
    return &g_map[type];
}

int caller_helper_ptr(int t)
{
    struct Ctor *c = map_get(t);
    return c->obtain(t);
}

int caller_direct(int t)
{
    return g_map[t].obtain(t);
}

/* 2. Tentative definition (no initializer) + runtime element stores (fx12). */
static void impl_a(int x);
static void impl_b(int x);

struct Handler {
    void (*fn)(int);
};

static struct Handler g_tbl[2];

void fill(int i)
{
    if (i) {
        g_tbl[i].fn = impl_a;
    } else {
        g_tbl[i].fn = impl_b;
    }
}

int run(int i)
{
    fill(i);
    g_tbl[i].fn(i);
    return 0;
}

static void impl_a(int x) { (void)x; }
static void impl_b(int x) { (void)x; }

/* 3. Local array with designated initializers (fx10). */
static int loc_a(int x) { return x; }
static int loc_b(int x) { return x; }

int caller_local(int t)
{
    struct Ctor tbl[2] = {
        [0] = { .obtain = loc_a },
        [1] = { .obtain = loc_b },
    };
    return tbl[t].obtain(t);
}
