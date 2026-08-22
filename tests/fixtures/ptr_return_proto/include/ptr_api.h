#ifndef PTR_API_H
#define PTR_API_H

struct Widget {
    int ready;
};

/* Pointer-returning prototype: must register a function, not a variable. */
struct Widget *WidgetGet(void);

#endif
