struct Base {
    virtual void hook();
    void go() { hook(); }
};

struct Derived : Base {
    void hook() override;
};

void Base::hook() {}
void Derived::hook() {}

static void drive(Base *p) { p->go(); }

namespace std {
template <typename T>
class shared_ptr {};
template <typename T>
class unique_ptr {};
template <typename T>
class weak_ptr {};
} // namespace std

struct Plugin {
    virtual void OnEvent();
    void OnEventProxy() { OnEvent(); }
};

void Plugin::OnEvent() {}

static void call_sp(std::shared_ptr<Plugin> p) { p->OnEventProxy(); }

static void call_sp_ref(const std::shared_ptr<Plugin> &p) { p->OnEventProxy(); }

static void call_up(std::unique_ptr<Plugin> p) { p->OnEventProxy(); }

static void call_wp(std::weak_ptr<Plugin> p) { p->OnEventProxy(); }

struct Holder {
    std::shared_ptr<Plugin> plugin_;
    void go() { plugin_->OnEvent(); }
};

static void drive_holder(Holder *h) { h->go(); }

struct Over {
    virtual void foo(int);
    virtual void foo(int, int);
};
struct OverD : Over {
    void foo(int) override;
    void foo(int, int) override;
};

void Over::foo(int) {}
void Over::foo(int, int) {}
void OverD::foo(int) {}
void OverD::foo(int, int) {}

static void call_unary(Over *p) { p->foo(1); }
static void call_binary(Over *p) { p->foo(1, 2); }

struct Event {};
struct Sink {
    bool consume(Event &event __UNUSED);
};
bool Sink::consume(Event &event __UNUSED) { return false; }

namespace ns {
void foo();
void foo_bar();
}
void ns::foo() {}
void ns::foo_bar() { ns::foo(); }
