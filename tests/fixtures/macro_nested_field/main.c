struct Leaf {
    int *slot;
};

struct Node {
    struct Leaf leaf;
};

#define NODE_SLOT(n) ((n)->leaf.slot)

void nested_macro_store(struct Node *n, int *v) {
    NODE_SLOT(n) = v;
}
