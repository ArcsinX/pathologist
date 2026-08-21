#define FEATURE 1

#if FEATURE
int active = 1;
#else
int dead = 2;
#endif

#if !FEATURE
int also_dead = 3;
#else
int also_active = 4;
#endif
