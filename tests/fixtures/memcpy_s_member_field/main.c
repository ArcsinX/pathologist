/*
 * Regression: memcpy_s(&drv->chipData, ..., chipData, ...) copies a struct
 * containing function pointers into a sub-field of the destination.
 * The MemCopy model must flow the source's pointees into the destination
 * variable so that GEP processing creates field cells and indirect calls
 * through the copied ops table resolve.
 */

typedef unsigned long size_t;
int memcpy_s(void *dest, size_t destsz, const void *src, size_t n);

struct Ops {
    void (*Enable)(void);
    void (*Disable)(void);
    int (*ReadData)(int);
};

struct ChipData {
    struct Ops ops;
    int value;
};

struct DriverData {
    struct ChipData chipData;
    int other;
};

static int ppg_enable_count = 0;
static int ppg_disable_count = 0;

void SetPpgEnable(void) { ppg_enable_count++; }
void SetPpgDisable(void) { ppg_disable_count++; }
int SetPpgReadData(int x) { return x; }

static void init_chip(struct ChipData *chip) {
    chip->ops.Enable = &SetPpgEnable;
    chip->ops.Disable = &SetPpgDisable;
    chip->ops.ReadData = &SetPpgReadData;
    chip->value = 42;
}

static void setup_driver(struct DriverData *drv, struct ChipData *chip) {
    init_chip(chip);
    memcpy_s(&drv->chipData, sizeof(struct ChipData), chip, sizeof(struct ChipData));
    drv->other = 1;
}

void test_indirect_through_memcpy_s(void) {
    struct ChipData src;
    struct DriverData drv;

    setup_driver(&drv, &src);

    /* These indirect calls must resolve through the memcpy_s model: */
    drv.chipData.ops.Enable();     /* -> SetPpgEnable */
    drv.chipData.ops.Disable();    /* -> SetPpgDisable */
    drv.chipData.ops.ReadData(0);  /* -> SetPpgReadData */
}
