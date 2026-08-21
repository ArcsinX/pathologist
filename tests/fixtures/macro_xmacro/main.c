#define FOR_EACH_OP(OP) OP(alpha) OP(beta)

#define DECLARE_OP(name) \
    struct name##_ctx { int slot; }; \
    void name##_handler(void);

#define DEFINE_OP(name) \
    struct name##_ctx name##_ctx; \
    void name##_handler(void) { name##_ctx.slot = 1; }

FOR_EACH_OP(DECLARE_OP)
FOR_EACH_OP(DEFINE_OP)

#define TABLE_ENTRY(name) name##_handler,

void (*op_table[])(void) = {
    FOR_EACH_OP(TABLE_ENTRY)
};

void driver(void) {
    op_table[0]();
}
