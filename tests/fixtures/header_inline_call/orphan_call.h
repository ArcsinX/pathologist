#ifndef ORPHAN_CALL_H
#define ORPHAN_CALL_H

extern void ExternalTarget(void);

static inline void HeaderOnlyCaller(void) {
    ExternalTarget();
}

#endif
