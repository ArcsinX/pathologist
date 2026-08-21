#define OUTER 1
#define INNER 0

#if OUTER
int outer_on = 1;
#if INNER
int inner_on = 2;
#else
int inner_off = 3;
#endif
#else
int outer_off = 4;
#endif
