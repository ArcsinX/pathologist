#include "ptr_api.h"

static struct Widget g_widget = { 1 };

struct Widget *WidgetGet(void)
{
    return &g_widget;
}
