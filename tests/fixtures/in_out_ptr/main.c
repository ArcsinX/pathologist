int global_x;

void init(int **pp) {
    *pp = &global_x;
}

void caller(void) {
    int *p;
    init(&p);
}
