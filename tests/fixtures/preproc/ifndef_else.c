#define GUARD

#ifdef GUARD
int guarded = 1;
#else
int unguarded = 0;
#endif

#ifndef GUARD
int missing = 0;
#else
int present = 1;
#endif
