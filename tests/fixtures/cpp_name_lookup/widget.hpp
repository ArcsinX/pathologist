#ifndef WIDGET_HPP
#define WIDGET_HPP

namespace kit {
struct Widget {
    int value() { return 1; }
};

void swap(Widget *a, Widget *b);
} // namespace kit

namespace util {
void helper();
int twice(int);
} // namespace util

namespace lib {
struct Count {
    int n;
};
void bump(Count *c);
} // namespace lib

namespace a {
namespace b {
int clamp(int v);
int go();
} // namespace b
} // namespace a

#endif
