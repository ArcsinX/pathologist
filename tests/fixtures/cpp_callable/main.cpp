static int target() { return 1; }

struct Holder {
    int (*cb)();
};

static void setup_field(Holder *h) { h->cb = target; }

static void call_field(Holder *h) { h->cb(); }

typedef int (*fn_t)();

static void call_local() {
    fn_t p = target;
    p();
}

static void call_lambda() {
    auto g = []() { return target(); };
    g();
}

struct Fn {
    void operator()() { target(); }
};

static void call_functor() {
    Fn f;
    f();
}

struct Wrap {
    Fn cb;
};

static void call_functor_field(Wrap *w) { w->cb(); }

namespace std {
template <typename T>
class function {};
} // namespace std

static void call_std_function() {
    std::function<int()> f = target;
    f();
}

struct WithFn {
    std::function<int()> getPluginObject;
};

static void setup_std_field(WithFn *w) { w->getPluginObject = target; }

static void call_std_field(WithFn *w) { w->getPluginObject(); }

namespace FileUtil {
bool Exists(const char *p);
}

static void check_exists() { FileUtil::Exists("x"); }

struct function {
    void operator()() { target(); }
};

static void call_bare_function_type() {
    function f;
    f();
}
