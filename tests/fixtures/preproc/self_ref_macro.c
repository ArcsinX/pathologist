/* Self-referential object macro (hiview PRIVATE_MESSAGE_TYPE pattern). */
#define PRIVATE_MESSAGE_TYPE \
        PRIVATE_MESSAGE_TYPE, \
        ENGINE_UPLOAD_READY_MSG

enum { PRIVATE_MESSAGE_TYPE };

#define MIN(a, b) ((a) < (b) ? (a) : (b))
int x = MIN(MIN(1, 2), 3);
