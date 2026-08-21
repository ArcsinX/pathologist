void callee(int *p);

void via_param(void (*cb)(int *), int *data) {
    cb(data);
}

void setup(void) {
    int x;
    via_param(callee, &x);
}
