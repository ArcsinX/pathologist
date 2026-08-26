/* caller.c — Exercises cross-struct field access via void* casts.

   CallWithOpsA receives a void* pointing to OpsA, casts to OpsA*,
   and calls ops->callback. Must resolve to CallbackImplA only.
   CallWithOpsB does the same with OpsB and ops->handler -> HandlerImplB.

   Before the cross-struct FieldId fix, both paths would see all fn
   pointers stored at positional index 0 in any struct, causing false
   positives across structs. */
#include "shared.h"

int CallWithOpsA(void *ops, int x) {
    struct OpsA *a = (struct OpsA *)ops;
    return a->callback(x);
}

int CallWithOpsB(void *ops, int x) {
    struct OpsB *b = (struct OpsB *)ops;
    return b->handler(x);
}

int CallBoth(int x) {
    void *pa, *pb;
    RegisterOpsA(&pa);
    RegisterOpsB(&pb);
    int r1 = CallWithOpsA(pa, x);
    int r2 = CallWithOpsB(pb, x);
    return r1 + r2;
}
