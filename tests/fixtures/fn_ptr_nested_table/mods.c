struct Mod {
    const char *name;
    int (*init)(int);
};

static int InitNet(int v)
{
    return v * 2;
}

static int InitFs(int v)
{
    return v * 3;
}

struct Mod g_modules[2] = {
    { .name = "net", .init = InitNet },
    { .name = "fs", .init = InitFs },
};

int CallMod(int i)
{
    struct Mod *m = &g_modules[i];
    return m->init(i);
}
