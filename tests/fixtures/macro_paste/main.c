#define CAT(a, b) a##b
#define HANDLER(name) void CAT(name, _handler)(void)

HANDLER(gamma)

void gamma_handler(void) {
}

void call_gamma(void) {
    gamma_handler();
}
