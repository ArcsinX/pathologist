void consume(int *p);

void provider(int *q) {
    consume(q);
}

void entry(void) {
    int value;
    provider(&value);
}
