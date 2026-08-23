/* Calls into code that has no definition under the analyzed root:
 * `ext_helper` is declared but defined elsewhere; `undeclared_stub` has no
 * prototype at all (implicit declaration). Both must be classified as
 * external callees, not left as unresolved indirect sites.
 * `helper` is the opposite case: undefined here but DEFINED in util.c —
 * cross-TU recovery must bind it to the real body. */

extern int ext_helper(int x);

int local_wrap(void)
{
    return ext_helper(7) + undeclared_stub(3);
}

int caller(void)
{
    return helper(5);
}
