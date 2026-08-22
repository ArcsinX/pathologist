#ifndef SHARED_H
#define SHARED_H

static inline int hdr_helper(int x) {
    return x + 1;
}

static inline int hdr_add(int a, int b) {
    return hdr_helper(a) + b;
}

#endif
