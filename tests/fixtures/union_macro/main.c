union Pun {
    int *p;
    float f;
};

#define UNION_P(u) ((u).p)

void union_macro_store(void) {
    union Pun u;
    int x;
    UNION_P(u) = &x;
}
