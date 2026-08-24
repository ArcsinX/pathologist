#define SHARED(T) int status_##T; \
    void (*ref)(struct T *);

#define WRAP_FIELDS(T) SHARED(T)

typedef struct {
    WRAP_FIELDS(Node)
} NodeWrap;

int done = 1;
