#include "ptr_api.h"

int CheckReady(void)
{
    struct Widget *w = WidgetGet();
    return w->ready;
}
