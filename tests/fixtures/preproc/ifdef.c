#define FEATURE 1

#ifdef FEATURE
int enabled = 1;
#else
int enabled = 0;
#endif

int get_enabled(void) {
    return enabled;
}
