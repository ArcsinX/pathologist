struct Ops {
    int (*fn)(int);
};

static int FnA(int v)
{
    return v + 1;
}

static int FnB(int v)
{
    return v + 2;
}

struct Ops tbl[2] = { { FnA }, { FnB } };

int CallTbl(int i)
{
    return tbl[i].fn(i);
}
