#ifndef OPS_H
#define OPS_H
struct Ops {
    void (*handler)(int *);
};
void target(int *p);
#endif
