#include <stdio.h>

void helper(int x) {
    (void)x;
}

int main(void) {
    helper(42);
    return 0;
}
