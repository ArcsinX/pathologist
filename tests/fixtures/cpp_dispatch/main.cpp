// Virtual inheritance (diamond), `final` class, and `final` method.

struct VBase {
    virtual int id();
};
struct Left : virtual VBase {
    int id() override;
};
struct Right : virtual VBase {};
struct Diamond : Left, Right {
    int id() override;
};

void VBase::id() {}
void Left::id() {}
void Diamond::id() {}

static void diamond_drive(VBase *p) { p->id(); }

struct Open {
    virtual int f();
};
struct Sealed final : Open {
    int f() override;
};
struct OpenSib : Open {
    int f() override;
};

void Open::f() {}
void Sealed::f() {}
void OpenSib::f() {}

static void sealed_drive(Sealed *p) { p->f(); }
static void open_drive(Open *p) { p->f(); }

struct MBase {
    virtual int g();
};
struct MMid : MBase {
    int g() final;
};
struct MLeaf : MMid {
    int other();
};

void MBase::g() {}
void MMid::g() {}
void MLeaf::other() {}

static void mid_drive(MMid *p) { p->g(); }
static void mbase_drive(MBase *p) { p->g(); }
