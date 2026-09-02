#include "widget.hpp"

void kit::swap(kit::Widget *a, kit::Widget *b) {
    (void)a;
    (void)b;
}

void util::helper() {}

int util::twice(int v) { return v * 2; }

void lib::bump(lib::Count *c) { (void)c; }

namespace a {
namespace b {
int clamp(int v) { return v; }

int go() { return clamp(1); }
} // namespace b
} // namespace a

// ADL: `swap(_a, _b)` resolves to `kit::swap` through the `kit::Widget *`
// argument types, even though `main` sits at global scope.
static void adl_drive() {
    kit::Widget x, y;
    kit::Widget *_a = &x;
    kit::Widget *_b = &y;
    swap(_a, _b);
}

// `using namespace util;` brings `helper` / `twice` into ordinary lookup.
using namespace util;

static void using_ns_drive() {
    helper();
    twice(3);
}

// `using lib::bump;` imports the specific qualified function.
using lib::bump;

static void using_member_drive(lib::Count *c) { bump(c); }

// A namespace-scoped `static` function imported via `using`: the import
// must resolve to the internal linkage definition in this file (when a
// global same-named overload exists, it must not win).
namespace import_static {
static void only(int v) { (void)v; }
} // namespace import_static
using import_static::only;
void only(double v) { (void)v; }
static void using_static_drive() { only(1); }

// Explicitly qualified calls still resolve (unchanged path).
static void qualified_drive() {
    util::helper();
    util::twice(2);
}

// Namespace-relative ordinary lookup from inside `a::b`: bare `clamp` finds
// `a::b::clamp` (innermost namespace) without qualification.
static void ns_drive() { a::b::go(); }

// ---- ADL / using edge cases ----

// Global `swap` overload with the same arity as `kit::swap`. Under
// may-analysis an unqualified `swap(_a, _b)` with `kit::Widget*` args must
// keep BOTH candidates (global `::swap` and ADL `kit::swap`).
void swap(kit::Widget *a, kit::Widget *b) {
    (void)a;
    (void)b;
}

// ADL nested member import: `using deep::inner::fold;` resolves a *nested*
// qualified name that is not a candidate under any ordinary/ADL namespace.
namespace deep {
namespace inner {
int fold(int v);
}
} // namespace deep

int deep::inner::fold(int v) { return v; }

// A `static` file-local bare function shadows ADL/global resolution.
static void shadowed(int v) { (void)v; }

// Namespace provided to `using namespace` inside a function body, so the
// directive set is function- (not file-) scoped.
namespace body {
void poke();
}
void body::poke() {}

// ADL may-approx: `swap(_a, _b)` must keep both the global `::swap` and the
// ADL `kit::swap` as candidates.
static void adl_may_approx() {
    kit::Widget x, y;
    kit::Widget *_a = &x;
    kit::Widget *_b = &y;
    swap(_a, _b);
}

// `using deep::inner::fold;` imports a nested qualified function.
using deep::inner::fold;
static void adl_nested_import(int v) { fold(v); }

// A `static` file-local must resolve over any ADL/global candidate of the
// same base name (file-scope internal linkage wins).
static void adl_static_shadow(int v) { shadowed(v); }

// `using namespace body;` scoped to a single function body.
static void adl_function_scoped_using() {
    using namespace body;
    poke();
}

// Same-named `poke` at global scope: `adl_function_scoped_using`'s directive
// must NOT leak here, so the global `poke` is the only candidate.
void poke() {}

// The leaked-directive guard: a function-body `using namespace` that would
// rank above this global overload must not rob the correct in-scope edge.
static void adl_using_no_leak() { poke(); }

// A namespace-block-scoped `using namespace`: written inside
// `scoped_use::inner`, it applies only to that block. `out_of_scope` (in the
// enclosing `scoped_use` namespace, declared after) must fall back to the
// in-scope global `tick(double)`; a TU-wide leak would instead collapse its
// `tick(1)` onto the better-ranking `boost_ish::tick(int)`.
namespace boost_ish {
void tick(int v);
}
void boost_ish::tick(int v) { (void)v; }
void tick(double v) { (void)v; }

namespace scoped_use {
namespace inner {
using namespace boost_ish;
static void in_scope() { tick(1); }
} // namespace inner
static void out_of_scope() { tick(1); }
} // namespace scoped_use

// Relative `using` targets: written inside an enclosing namespace, `detail`
// refers to the *enclosing* `relns::detail` (C++ resolves the first segment
// against the enclosing scope), which takes precedence over the global
// `detail`. The directive lives in a host namespace and the call is nested
// below it, so the call relies on the directive (not on its own enclosing
// lookup) to reach `relns::detail::bump` — `cpp_relative_using_..._finds_
// enclosing_namespace` pin this down.
namespace detail {
void bump(int v);
}
void detail::bump(int v) { (void)v; }
namespace relns {
namespace detail {
void bump(int v);
}
void detail::bump(int v) { (void)v; }
namespace directive_host {
using namespace detail;
namespace user {
static void drive_ns() { bump(1); }
} // namespace user
} // namespace directive_host
namespace import_host {
using detail::bump;
namespace user {
static void drive_import() { bump(1); }
} // namespace user
} // namespace import_host
} // namespace relns

// A `::`-spelled global definition written inside a namespace block must
// register at global scope (`ns::x` prefix must NOT be prepended).
namespace global_block {
void ::qualified_global() {}
static void caller() { qualified_global(); }
} // namespace global_block

// Inner-block `using namespace`: written inside an `if` body, it applies only
// to that block, not the enclosing function body. The call inside the block
// resolves through `innerlib::g` (over-approximating the global `g` too); the
// sibling call after the block (same function) must NOT see the directive and
// must stay on the global `g`. A directive leaked to the whole function would
// instead pull `innerlib::g` into the sibling call site too (over-approx that
// can collapse the ranking and rob the correct in-scope edge).
namespace innerlib {
void g();
}
void innerlib::g() {}
void g() {}

static void inner_block_using_scoped() {
    if (true) {
        using namespace innerlib;
        g();
    }
    g();
}

// Leading-`::` ADL tag: `::kit::Widget` (global-scope spelling of the same
// type) must still derive ADL namespace `kit` (not `::kit`), so the
// unqualified `swap` finds `kit::swap`.
namespace kit {
struct LeadWidget {};
void lead_swap(LeadWidget *w);
}
void kit::lead_swap(kit::LeadWidget *w) { (void)w; }

static void adl_leading_global_scope_tag() {
    ::kit::LeadWidget w;
    ::kit::LeadWidget *_w = &w;
    lead_swap(_w);
}

// Hiding rule, outer rung: an inner-namespace declaration must shadow a
// global-namespace one of the same name. `hide::g`'s bare `f(1)` must
// resolve to `hide::f` (the global `::f(int)` is shadowed and dropped).
void f(int);
namespace hide {
void f(double v) { (void)v; }
void g() { f(1); }
} // namespace hide

// Hiding rule for internal linkage: a nested namespace declaration must
// also shadow a global file-scope static of the same name. `hidesf::g`'s
// bare `sf(1)` must resolve to `hidesf::sf` (the global static `::sf` is
// dropped, not the wrong single answer).
static void sf(int v) { (void)v; }
namespace hidesf {
void sf(double v) { (void)v; }
void g() { sf(1); }
} // namespace hidesf

