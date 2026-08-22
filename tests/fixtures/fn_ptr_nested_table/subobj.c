struct Svc {
    int (*Dispatch)(int);
    int (*TestEntry)(int);
};

struct Dev {
    struct Svc *service;
};

static int EntryFn(int v)
{
    return v + 100;
}

static int RealDispatch(int v)
{
    return v + 200;
}

static struct Svc inst;
static struct Dev dev;

void Bind(void)
{
    dev.service = &inst.service;
    inst.TestEntry = EntryFn;
    inst.Dispatch = RealDispatch;
}

int InvokeTest(int x)
{
    return inst.TestEntry(x);
}

int CoreRun(int x)
{
    struct Svc *m = dev.service;
    if (m == 0 || m->Dispatch == 0) {
        return -1;
    }
    return m->Dispatch(x);
}
