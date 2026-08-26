/* shared.h — Shared struct definitions for cross-struct test */
#ifndef SHARED_H
#define SHARED_H

/* Both structs have a function pointer at field index 0, but with
   different field names ("callback" vs "handler"). This is the exact
   pattern that caused 140 false positives for HdfSbufReadBuffer. */
struct OpsA {
    int (*callback)(int);
    int data;
};

struct OpsB {
    int (*handler)(int);
    int count;
};

void RegisterOpsA(void **out);
void RegisterOpsB(void **out);

#endif
