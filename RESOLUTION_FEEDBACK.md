# TRACE call-site resolution evaluation — `~/drivers_hdf_core`

Date: 2026-08-23 · Build: workspace @ uncommitted HEAD (local mods in pag/solver/types/lower/merge).
Scope: direct + indirect (function-pointer) call-site resolution; FP/FN reduction opportunities.

## 0. TL;DR

| Area | Verdict |
|---|---|
| Indirect-call precision | Strong. Flagship TP proofs resolve to *exactly* their initializer sets (HDMI 15/15). |
| Whole-tree scale | ~16s end-to-end at the default 300k-pop budget (was: non-converging hang); full fn-ptr site coverage, deterministic results; convergence design work still open (P0-1), though noise-fuel removal made late-stage growth far cheaper. |
| Preproc cache soundness | **Silent per-TU definition loss** from warm-pass shared guards × frozen expansion cache (P0-2) — verified cause of ≥11 concrete FNs. |
| Include-dir ergonomics | Wrong-platform twins picked by `-I` order (P0-3); headers outside dirs named `include` easily missed (P0-4). |
| Noise | Small, classifiable: C++-header phantoms, log-macro artifacts, libc bare names. |

Evaluation DB for all numbers: `/tmp/hdf_fw3.db` (root=`framework`, all 205 header dirs as `-I`, `-D __LITEOS__`, `--jobs 8`, 3.9 s):
**6313 functions · 22285 direct sites · 15892 indirect sites · 26067 resolved edges.**

## 1. Method

Probes built against public APIs:
- `/tmp/opencode/ppraw` — raw `trace_preproc::preprocess_file` dump (`-I`, `-D`).
- `/tmp/opencode/ppprobe` — replicates the exact CLI pipeline (`discover_source_files` → `IncludeGraph::build` → warm pass w/ `with_accumulate_macros(true)` → frozen-cache TU indexing); switches: `SKIP_WARM=1`, `MODE=WARM_SHARED_ONLY|NO_SHARED`, `TRACE_DUMP=<file>`.
- SQLite queries on `call_sites` / `call_edges` / `functions` / `files`.
- Minimal fixtures `/tmp/opencode/fx1..fx4`.

Evaluation DB command:
```bash
target/release/trace analyze ~/drivers_hdf_core/framework -o /tmp/hdf_fw3.db \
  $(find ~/drivers_hdf_core -type f -name '*.h' -exec dirname {} \; | sort -u | sed 's/^/--include /') \
  -D __LITEOS__ --jobs 8
```

## 2. Blocking defects

### P0-1 — Solver does not converge on the whole tree

`TRACE_SOLVER_STATS=1 target/release/trace analyze ~/drivers_hdf_core ...` (killed >15 min; `/tmp/full_run.log`):

| pop | copy work | constraints | max_pts |
|-----|-----------|-------------|---------|
| 0   | 11M       | 42,733      | 162     |
| ~150K | 57M     | 57K         | 1,024   |
| ~300K | 141M    | 65K         | 2,560   |
| ~450K | 292M    | 70K         | 4,096   |
| ~600K | 495M    | 73K         | 5,120   |
| ~900K | 827M    | 76,101      | 6,919   |

Mechanism: dynamic param-copy wiring during solve (`wire_params` / `ensure_param_copy`,
crates/trace-analysis/src/solver.rs) grows the constraint set *while solving*;
newly discovered FieldSummary-driven indirect targets add more params, which add more
copies — superlinear feedback. Late-phase throughput ≈ 100K pops / 8.5 min.
Recommendations: cap/wire params once at PAG build; budget + report partial results
with diagnostics; consider differential re-solve instead of incremental rewiring.

### P0-2 — Warm-pass shared guards × frozen expansion cache: silent per-TU definition loss

**Symptom.** `DListRemove` (static inline, `interfaces/inner_api/utils/hdf_dlist.h:32`, guard
`HDF_LIST_H`) resolves in most TUs (60 sites with edges) but is **unresolved at 11 sites** in fw3.db:

| file | lines |
|---|---|
| framework/model/sensor/driver/common/src/sensor_device_manager.c | 1401, 1726 |
| framework/support/platform/src/regulator/regulator_core.c | 1797, 1833 |
| framework/support/platform/src/regulator/regulator_tree_mgr.c | 801, 1150 |
| framework/support/platform/src/timer/timer_core.c | 1690, 1752 |
| framework/test/unittest/model/usb/device/src/usb_device_lite_cdcacm_test.c | 954, 1124, 1193 |

No diagnostic is emitted; edges are simply missing.

**Proof chain (each step reproduced).**

1. Raw preprocessing of the TU with the same `-I` set **contains** the definitions:
   `ppraw sensor_device_manager.c` → `void DListRemove` present. So include *resolution* is fine.
2. Replicating the exact CLI pipeline (`ppprobe`): TU output **lacks** them, deterministically.
   - `SKIP_WARM=1` → present ⇒ warm pass is necessary for the loss.
   - `MODE=WARM_SHARED_ONLY` (warm ran; shared macros used; no cache) → absent ⇒ shared macro table alone suffices.
3. After warm, `HDF_LIST_H ∈ shared_macros` (table size 1641). ~200 warmed headers expand
   `hdf_dlist.h` inline during their own warm-up ("leakers": `framework/include/osal.h:13`,
   `hdf_syscall_adapter.h`, `osal.h` consumers, etc.).
4. Minimal isolation: standalone preprocess of `#include "sensor_device_manager.h"` with `-D HDF_LIST_H=1`
   reproduces the loss; `-D OSAL_H=1` does **not** (route is via
   `interfaces/inner_api/host/shared/hdf_device_desc.h`, not osal.h).
5. TU-phase replay log: only 3 cache entries are served (`sensor_device_manager.h` textlen=14258
   **has_dlist=false**, `interfaces/inner_api/osal/shared/osal_mem.h` textlen=179,
   `sensor_platform_if.h`); zero inline header expansions occur, so there is no recovery path.
6. Success asymmetry: TUs that resolve reach one of the **115** cached entries whose baked text
   contains dlist content.

**Root cause.** The warm pass shares one macro table across headers
(`PreprocessOptions::shared_macros` + `with_accumulate_macros(true)`,
crates/trace-preproc/src/options.rs:66,71). A header warmed *after* any leaker sees guard
macros already defined, so nested includes emit zero content — and that starved expansion is
what gets frozen into its `IncludeExpansion` (crates/trace-preproc/src/preprocessor.rs:255–270)
and replayed verbatim to every TU (frozen replay at preprocessor.rs:190–209).

Secondary conflation: `entry.files` records paths inserted into `included_guard`
(preprocessor.rs:223) even when their token expansion was fully skipped by a guard — i.e.
"claimed" without content. Not the trigger here (poison scan found 0), but the same class of bug.

**Impact class.** Any static-inline function/type pulled through a header that was warmed after a
twin defining its guard can vanish from arbitrary subsets of TUs — silently, and dependent on
header warming order. This is both an FN machine and a determinism hazard across tree shapes.

**Fix directions.**
1. Stamp each `IncludeExpansion` with the macro-table state (or just guard names) it was created
   under; on frozen replay, if current state differs, expand fresh instead of replaying.
2. Record `entry.files` only when content was actually emitted.
3. Emit a diagnostic when a resolved `#include` yields zero tokens due to an already-defined guard
   (distinguishable from genuinely empty headers).
4. Alternatively drop cross-header guard accumulation for the *cache-building* pass entirely;
   per-TU C semantics then guarantee correctness (at some perf cost).

### P0-3 — Analysis root ≠ `-I` tree copy → starvation

Analyzing `/tmp/opencode/hdffull` (copy) while passing `-I ~/drivers_hdf_core/...`: warm-pass
guards accumulate globally while the expansion cache keys on canonical path; inline-expanded twin
headers hit pre-defined guards and expand empty. Result: 8 headers never interned
(`framework/utils/include/{hcs_blob_if,hcs_parser,hdf_message_looper,hdf_message_task,hdf_task_queue,hdf_thread_ex,osal_message,osal_msg_queue}.h`)
and their static-inline functions (`HcsAlignSize`, …) disappear — again with no error.
Recommendation: warn whenever a resolved include lies outside the analysis root.

### P0-4 — Platform twins selected by `-I` order, not by target

`sensor_device_manager.h`'s cached entry claims
`adapter/khdf/hongmeng/osal/include/{hdf_types,hdf_workqueue}.h` — the **hongmeng** OSAL wins over
liteos purely because it sorts first among `-I` dirs, despite `-D __LITEOS__`. Wrong-platform
content risks spurious/missing decls corpus-wide. Recommendation: variant-aware resolution
(e.g., prefer dirs matching active platform defines, or explicit `--platform-dir` mapping).

## 3. Resolution quality snapshot (/tmp/hdf_fw3.db)

Unresolved indirect sites: 13,651 bare-name · 534 field-path · 29 subscript.

### 3.1 True-positive proofs

| Site | Result |
|---|---|
| `framework/support/platform/src/hdmi/hdmi_dispatch.c:289` `HdmiIoDispatch` → `cmd->Dispatch(...)` | **Exactly 15 indirect targets = the 15 `dispatchFunc[]` initializer entries** (HdmiCmdOpen/Close/Start/Stop/ReadSinkEdid/VideoAttrSet/AudioAttrSet/HdrAttrSet/DeepColor{Get,Set}/AvmuteSet/InfoFrame{Get,Set}/{Un}RegisterHpdCallbackFunc). No FP targets. |
| Sensor `deviceInfo->ops->Enable` | Resolves to all `Set*Enable` implementations across drivers — correct may-analysis (each driver stores its own ops). |
| Dispatcher chains `dispatcher->Dispatch` — `core/shared/src/hdf_io_service.c:36`, `adapter/vnode/src/hdf_vnode_adapter.c:204,236,370`, `core/common/src/hdf_device_node_ext.c:136` | Resolve once inner_api dirs included (fw2/fw3). |
| `DListRemove` elsewhere | 60 sites resolved; static-inline + internal-linkage handling works when content survives (see P0-2). |

### 3.2 False-negative classes

| Class | Evidence | Fix direction |
|---|---|---|
| F1 Preproc-cache loss (P0-2) | 11 DListRemove sites above; likely broader symbol loss | see P0-2 fixes |
| F2 Declaration-order initializers | fx1: `.field = LaterFn` where `LaterFn` is defined later w/o forward decl → lowering emits dangling `GepField`, no Store (`resolve_function_named`, crates/trace-parse/src/lower.rs:1531; `lower_designated_initializer` :948) | two-pass lowering / deferred callee resolution for initializers |
| F3 libc/system externals | bare-name unresolved 13.6K cluster: strcmp/ioctl/poll/strerror… (no system headers in corpus) | seed external registry from `trace-analysis/src/summaries.rs` names, mark "known-external" so they don't pollute FN metrics |
| F4 Out-of-corpus symbols | `DCacheFlushRange`/`DCacheInvRange` (mtd_core.h:392,406); `HdfPower` op slots unassigned anywhere in-corpus (correctly unresolved) | tag as out-of-tree rather than unresolved |
| F5 Empty struct layouts | vnode probe: `kDispatcher : Struct {name:"HdfIoDispatcher", fields:[]}` when definition absent from TU text | ensure header-origin layouts merge before GEP lowering or fall back to FieldSummary eagerly |

### 3.3 False-positive / noise classes

| Class | Evidence | Fix direction |
|---|---|---|
| N1 Macro-expansion artifacts as call sites | `can_test.c` HDF_LOGE expansions stored as callee_text like `["E" "/" "HDF_LOG_TAG" "] " ...` (10+6+… sites) | suppress call-site extraction inside known log-wrapper expansions or filter non-identifier callee_text |
| N2 C++ headers parsed as C | `framework/tools/hdi-gen/parser/parser.h:126` `mPtr->AddRef/Release`, `members_->size`; hc-gen & fuzztest `random.h` phantoms | detect/skip C++-only headers (class/template/namespace tokens) |

## 4. Prioritized recommendations

1. **P0-2 fix** (cache stamping/invalidation + files-conflation + diagnostics) — removes silent FNs.
2. **P0-1 solver budgeting**: stop growing constraints mid-solve; report partial results + stats summary in export.
3. Include diagnostics: warn on outside-root resolutions (P0-3), on empty-guard expansions (P0-2.3), on ambiguous basename matches across platform variants (P0-4).
4. External-symbol registry seeding (F3/F4) to separate "unknown" from "external".
5. Lowering: deferred initializer callee resolution (F2); eager FieldSummary fallback for layout-less structs (F5).
6. Noise filters N1/N2.

## 5. Appendix — repro commands

```bash
# raw preproc check (should contain defs)
/tmp/opencode/ppraw <tu.c> $(find ~/drivers_hdf_core -type f -name '*.h' -exec dirname {} \; | sed 's/^/-I /' | sort -u | tr '\n' ' ') -D __LITEOS__ | grep -c 'void DListRemove'

# pipeline replication with stage switches
cd /tmp/opencode/ppprobe
SKIP_WARM=1 ./target/release/ppprobe ~/drivers_hdf_core/framework $ALLD -D __LITEOS__ --target <tu.c>   # present=true
MODE=WARM_SHARED_ONLY ./target/release/ppprobe ... --target <tu.c>                                      # present=false

# DB queries
sqlite3 /tmp/hdf_fw3.db "SELECT f.path,s.line FROM call_sites s JOIN functions fn ON fn.id=s.caller_fn_id JOIN files f ON f.id=fn.file_id WHERE s.callee_text='DListRemove' AND NOT EXISTS(SELECT 1 FROM call_edges e WHERE e.call_site_id=s.id);"
```

Temporary instrumentation added during this investigation to
`crates/trace-preproc/src/preprocessor.rs` (env-gated hit/inline logs) has been **fully reverted**;
workspace builds clean and `cargo test -p trace-preproc` passes.

## 6. Appendix — fix status (same day, post-feedback)

All P0-2-adjacent defects fixed in-tree; verification DB `/tmp/hdf_fw6.db` (identical command, fw3 → fw6):

| Metric | fw3 (before) | fw6 (after) |
|---|---|---|
| Direct call sites | 22285 | 22454 (+169) |
| Indirect call sites | 15892 | 15684 (−208: reclassified as direct) |
| Resolved edges | 26067 | 26165 (+98) |
| Unresolved indirect sites | 14185 | 14046 (−139) |
| DListRemove FN sites | 11 | **3** (remaining TUs never see any definition — implicit-decl territory, faithful) |
| `DListRemove` direct edges | 60 | 68 |
| HDMI `HdmiIoDispatch` TP | 15/15 indirect | unchanged (16th target = direct `DealFormat`) |

Fixes landed:

1. **Warm-pass guard starvation (P0-2 core)** — `lower.rs`: per-header fresh macro tables seeded from cmdline defines; union table for TUs; dedup via expansion cache. Unit-tested (`guard_skipped_include_not_claimed_and_warned`, `frozen_cache_does_not_warn_on_guard_skip`).
2. **Cache-entry conflation** — `preprocessor.rs`: per-file emitted-byte tracking; zero-emission files not claimed by `IncludeExpansion.files`; visible Warning on guard-skipped cached headers during non-frozen phases.
3. **Cache self-containment (root cause of residual FNs)** — flat entries froze without nested content when a guard-skip had already emitted that content upstream in the same run; consumers routed through such an entry lost the definitions permanently. Fix: guard-skip now re-splices the cached text (`splice_cached`); duplicate definitions are harmless (merge dedup). This — not the shared warm table alone — was the dominant loss mechanism.
4. **Raw-source fallback discarded preprocessed output** — `index_cache.rs`: a `preprocess stopped` warning inside ONE nested header caused the whole TU's output to be dropped in favor of raw source (328/440 TUs on this tree parsed unexpanded macros with zero header content). Indexing now keeps the truncated prefix; stop messages name the responsible file.
5. **Later-defined function references (F2 partial)** — `PendingFnRef` deferred resolution for designated-init stores, RHS idents, address-of idents, and returns; fixture `tests/fixtures/later_defined_init/`.
6. **N1 noise filter** — call sites whose callee text embeds string literals skipped (whitespace is legitimate in preprocessed callee text; only quotes filtered).
7. **P0-1 mitigation (default-on)** — solver stops at a deterministic 300k-pop budget by default (framework converges at ~42k). Whole-tree wall time dropped from >15min-hang to ~16s end-to-end; two factors: the cap, plus removing the noise-class call sites that had been feeding phantom call-target nodes into the worklist (see item 10). 300k is empirically the coverage knee on this tree: every genuine fn-pointer-deref site has ≥1 target by then (the old binary left 4 uncovered even at 400k), while past it returns collapse sharply (+176s for +4% indirect edges from 500k→700k). `TRACE_SOLVE_BUDGET_POPS=<n>` overrides; `=0` restores unlimited.
8. **P0-3 visibility** — CLI warns when include paths lie outside the analysis root (50 dirs in this corpus).
9. **Validation methodology corrected** — project headers resolve without manual `-I` for whole-tree roots (`discover_include_dirs`: header parent dirs + `include/` dirs + unique-basename fallback). Manual `-I` is only appropriate for outside-root/system deps and platform-twin selection (documented in `docs/PREPROCESSOR.md`).
10. **External-callee reclassification** — plain-identifier calls to functions with no definition under the analyzed root were polluting the indirect bucket (19 066 of 19 070 zero-target "indirect" sites whole-tree; 14 372 alone were `udk_log`, a logging backend referenced only inside a macro body). Fix: synthesized bodyless `functions` rows (`is_defined = 0`) + new `call_edges.resolution = 'external'`; prototype-only callees reclassify to external as well (edge to an undefined target). Param wiring is skipped only when the callee declares no formals, so arg-flow into prototyped interfaces is preserved. Schema/docs updated (`SQLITE_SCHEMA.md`, `README.md`, `ANALYSIS.md`).
11. **Indirect-edge audit (post-reclassification)** — whole-tree site coverage preserved vs the pre-change binary (1404 vs 1406 indirect sites). Of 23 sites that flipped indirect→external, 22 carried provably polluted old targets (`udk_log` sites attributed `HcsGetUint32` — an impossible flow from shared-temp pts pollution); 1 exposed a **pre-existing lowering defect**: field callees inside doubly-substituted test macros (`LONGS_EQUAL_RETURN(HDF_SUCCESS, g_driverEntry->Init(...))`) degrade to bare `Init` with the base object dropped, so the site loses its faithful `g_driverEntry->Init` resolution. Tracked as open item; affects macro-doubled field expressions only.

Not addressed (unchanged verdicts): full solver convergence (P0-1 design work), platform-twin selection by `-I` order (P0-4), C++ phantom noise (N2), field-path FNs dominated by genuinely-untyped callback tables (F3/F4).

Post-fix review round found and fixed two residual defects: pending-reference retry now falls back to variables (tentative globals defined after use), and `typedef ret (*Name)(...)` aliases resolve to pointer-to-function instead of function-returning-pointer. Corpus metrics unchanged by these fixes; determinism verified (two independent parallel runs produce identical call-edge and arg-flow sets); dispatch-table TPs reconfirmed (sensor cmd table exactly its 6 initializers, light table its 3, HDMI 15).
12. **Review-round fixes** — (a) HIGH: `finalize_extern_callees` originally synthesized externs over names that DO exist tree-wide, orphaning cross-TU definitions lacking local prototypes (solver name-recovery bypassed). Fixed with a `resolve_function` guard + regression fixture (`extern_call/util.c`: `caller→helper` must stay Direct to the defined body). (b) Solver hot-loop env lookup hoisted; redundant store-target clone removed; CLI ambiguous-edge grouping documented; warning quoting fixed.
13. **HDMI TP reconciliation after reclassification** — whole-tree runs unchanged: `HdmiIoDispatch` resolves 16 distinct fn-ptr/direct targets including a now-verified **direct edge to the real `DealFormat` body** in `adapter/khdf/hongmeng/osal/src/osal_deal_log_format.c`. Framework-subroot runs show 15 because `DealFormat`'s defining TU lies outside that root — the edge correctly becomes `external` instead of pretending the prototype is a definition. Final whole-tree metrics @300k default: 72983 edges (28978 direct / 25186 indirect / 18819 external), 51351 arg-flow, deterministic across runs; framework corpus 84/84 tests green.

## 7. Round 3 — 2026-08-24: array-table FN/FP fixes (designated inits, tentative arrays)

Scope: fresh whole-tree evaluation + minimal-fixture bisection of indirect-call resolution.
Builds compared on identical inputs (`-D __LITEOS__ --jobs 8`, default 200k-pop budget):
`old` = committed HEAD c699933, `new` = this round's fixes. DBs `/tmp/opencode/hdf_old.db`,
`/tmp/opencode/hdf_det1.db`. Note the default budget constant is now 200_000 (docs above
mention 300k); site coverage is budget-insensitive here (500 vs 495 unresolved @200k/@300k),
the budget only affects how many *extra may-targets* late pops add.

### 7.0 TL;DR

| Metric (whole tree) | old | new | Δ |
|---|---|---|---|
| Direct edges / sites | 28976 / unchanged | 28976 | — |
| Indirect resolved edges | 6485 | 6494 | −101 FP · +93 FN-fixed TP · +17 mixed |
| Unresolved indirect sites | 500 | 495 | −5 (all five now resolve correctly) |
| Arg-flow fn-ptr actuals | 12 | 12 | unchanged |
| Tests | 84 | **85** pass | new regression fixture |

All three defect classes were found by minimal fixtures first, then confirmed corpus-wide.

### 7.1 D1 (FN, blocking) — initializer-less array declarations dropped entirely

`lower_declaration` matched `"declarator" | "pointer_declarator" | "function_declarator"`
but not `array_declarator` (tree-sitter emits it as a *direct* child for `T arr[N];`
without initializer; with an initializer the child is `init_declarator`, which is why
initialized tables worked). Any tentative array definition therefore produced **no
variable at all** — globals and locals alike:

```c
static int g_noinit[4];          // missing from IR
int use(int i) { int l[4]; l[i]=1; return g_noinit[i]; }   // l missing too
static int g_init[4] = {1};      // present (init_declarator path)
```

Proof chain: fixture `tests/fixtures/array_table_designated` pre-fix DB had no row for
`g_tbl`; post-fix it exists and element stores resolve. This one-line arm addition also
closes the previously-open §6.11 item (field callees inside doubly-substituted test
macros degrading to bare names): `LONGS_EQUAL_RETURN(0, g_driverEntry.Init(7))` with a
tentative `g_driverEntry` now lowers the full field path and resolves `impl_init`
(fixture fx21).

Corpus impact: every runtime-populated dispatch table declared without an initializer
(e.g. `static struct Handler g_tbl[2];` + `g_tbl[i].fn = f`) was invisible to the solver.

### 7.2 D2 (FN) — designated-initializer array members lost

For `[i] = { .fn = Fn }` tables, `lower_designated_initializer` saw the
`subscript_designator` child first and mistook it for the pair's *value* (it only
excluded `field_designator`), so nothing was emitted and nested lists were never
recursed into. Positional tables (`{ { Fn }, ... }`) worked, which is why earlier
TP proofs (HDMI/sensor) never caught this.

Fix: subscript designators are skipped (index-insensitive per IR contract), and list
values recurse through `lower_initializer_list` chaining GEPs for any field designators,
landing in the existing precise `emit_field_value_store` path (AddrOfFn, pending refs
for later-defined callees, CallReturn all preserved).

Newly-resolved corpus sites (all verified against source):

| Site (source anchor) | Targets now | Proof |
|---|---|---|
| `framework/core/shared/src/hdf_object_manager.c:16` `targetCreator->Create()` | exactly the 18 `*Create` fns | table `framework/core/common/src/devlite_object_config.c:23–59`: designated `[OBJ_ID] = {.Create=…, .Release=…}` |
| `hdf_object_manager.c:34` `targetCreator->Release(object)` | exactly the 13 non-NULL `*Release` fns | same table (two entries have `.Release = NULL` and are absent from targets — correct) |
| `framework/utils/src/hdf_sbuf.c:405` `constructor->obtain(capacity)` | exactly `{SbufObtainRaw, SbufObtainIpc, SbufObtainIpcHw}` | `g_sbufConstructorMap` designated table at `hdf_sbuf.c:55` |
| `hdf_sbuf.c:467` `constructor->bind(base, size)` | exactly the 3 `Sbuf*Bind*` fns | same table |
| wifi sta cmd dispatch (`HandleRequestMessage`, `nodes/local_node.c:32`) → `targetService->ExecRequestMsg` chain | 56 `WifiCmd*` handlers | macro-built cmd table, e.g. `sta.c:631 DUEMessage(CMD_STA_SCAN, WifiCmdScan, 0)` |
| `power_state_token.c:53` `listener->Suspend(...)`, `:31 Resume`, Doze variants | exactly `{HdfPmHdfTestSuspend, HdfPmSampleSuspend}` etc. | runtime stores `hdf_pm_driver_test.c:223–228 …listener.powerListener.Suspend = HdfPmSampleSuspend` |

The HDMI flagship proof is unchanged: `HdmiIoDispatch` still resolves its 15 positional
`dispatchFunc[]` entries exactly (positional path untouched).

### 7.3 D3 (FP) — ArrayFnMember merged fields across designated tables

Because D2's fallback pushed designated members as field-less `ArrayFnMember` facts,
any load of *any* member of such a table observed *every* listed function:
`constructor->obtain()` targeted the three `.bind` functions too, and
`targetCreator->Create()`/`->Release()` each saw both families (31 bogus edges on those
two sites alone). Fix: members written via a field designator lower as precise
`GepField`+`Store` chains (identical shape to runtime element stores), so they feed only
loads of that field. Purely positional lists keep the documented merged-blob semantics
(`docs/ANALYSIS.md` updated).

Corpus-wide effect (old → new, per-site target-set diff): **101 polluted edges removed
across 10 sites**, spot-proofs that removals are correct:

- `GpioCntlrRead` (`gpio_core.c:67` `cntlr->ops->read`) lost 13 targets incl.
  `EmmcTestEntry` — which is only ever stored to `tester->TestEntry`
  (`emmc_test.c:150`), never to any `.read`.
- `DispatchAddDevice` lost `GpioDevSetDir`/`LinuxGpioSetDir` (only ever stored to
  `.setDir`: `gpio_wm.c:107`, `gpio_bes.c:130`, …) and `DevSvcManager{Stub,Ext}Start`
  (only to `.StartService`: `devsvc_manager_stub.c:708`).
- sbuf/object-manager Create↔Release and obtain↔bind cross-field splits (§7.2 table).
- `DevSvcManagerStubAddService`, `SetPpgOption`, `DevHostServiceFullOpsDevice` lost
  unrelated test/dispatch entries likewise.

Net new-edge audit: +93 TP (§7.2) − 101 FP + 17 at seven sites; of those 17, twelve are
verified-true pm-hook flows (above) and five are the known instance-insensitivity class
described in §7.5.

### 7.4 Regression safety

- New integration fixture `tests/fixtures/array_table_designated` + test
  `array_table_designated_init_resolves_targets` covers: global designated tables (helper-ptr
  and direct access), tentative arrays with runtime stores, local designated tables.
  Minimal probes also re-verified: plain fn-ptr arrays, cast-wrapped initializers,
  mixed `.fn/.arg` designated elements, later-defined callees.
- Workspace: 85/85 tests pass; two consecutive full-tree runs byte-identical edge sets
  (determinism holds for both binaries; an apparent nondeterminism mid-session was a
  stale-binary artifact of the A/B procedure, see §7.6).

### 7.5 Remaining known imprecision (unchanged verdicts, new evidence)

1. `servstat_listener.c:57` `stub->listener.callback(...)` gains {`DevSvcManagerExtStart`,
   `DevSvcManagerStubStart`, `GpioDevSetDir`, `GpioManagerAdd`, `LinuxGpioSetDir`} — none
   stored to any `.callback`. Mechanism: wrong-type pointer casts make pts(stub) contain
   foreign objects; field ids are then applied by offset onto alien layouts, plus
   instance-insensitive summaries. Pre-existing class, surfaced within budget by reshuffled
   propagation; would need type-aware field locations to fix.
2. Same-struct-type file-static tables merge across TUs through `(type, field)` summary
   locs: sensor `DispatchCmdHandle` sees the light driver's 3 table entries (9 targets;
   identical in old binary — not a regression). Tightening statics to per-var field locs
   is a candidate precision win.
3. Remaining 463 unresolved field-path sites are dominated by genuinely out-of-corpus
   implementations — verified samples: HDMI `audioPathEnable` (never assigned),
   `macChipDriver->ethMacOps` (only NULL-checked), sensor `AccelRegisterChipOps` (interface
   never called in-tree), `g_modules[i].deinit` (all entries `.deinit = NULL`). These are
   faithful results, not FNs; tagging them "out-of-tree" remains the right UX fix.
4. C++ phantom noise (`mPtr->AddRef`, `autoptr.h operator->`) and the `(vaddr_t)(uintptr_t)`
   cast-artifact callee text persist (N1/N2 classes from §3.3).

### 7.6 Methodology notes

- A/B builds via `git stash` must copy binaries to explicit paths *and* rebuild after
  `stash pop`; running `target/release/trace` afterwards silently exercises the wrong
  build (this produced one false determinism alarm and phantom fixture failures during
  the session).
- `call_sites.line` for TU-local sites is the *preprocessed* coordinate (schema-correct
  but easy to misread: `wifi_base.c:23984` in a 1813-line file). Feedback anchors above
  give source lines instead; consider exporting both (e.g. `pp_line`) next schema rev.

### 7.7 Repro commands

```bash
# evaluation DBs
target/release/trace analyze ~/drivers_hdf_core -o /tmp/opencode/hdf_new.db --jobs 8 -D __LITEOS__
# newly-resolved dispatch proofs (compare old/new)
sqlite3 /tmp/opencode/hdf_new.db "SELECT cs.line, group_concat(c.name) FROM call_edges e
 JOIN call_sites cs ON cs.id=e.call_site_id JOIN functions c ON c.id=e.callee_fn_id
 JOIN functions caller ON caller.id=cs.caller_fn_id WHERE caller.name='HdfObjectManagerGetObject'
 AND e.resolution='indirect' GROUP BY cs.id;"
# FP-removal proof: obtain/bind no longer cross-contaminated
sqlite3 /tmp/opencode/hdf_new.db "SELECT cs.line, group_concat(c.name) FROM call_edges e
 JOIN call_sites cs ON cs.id=e.call_site_id JOIN functions c ON c.id=e.callee_fn_id
 JOIN files f ON f.id=cs.file_id WHERE cs.line IN (893,944) AND f.path LIKE '%utils/src/hdf_sbuf.c'
 GROUP BY cs.line;"
cargo test --workspace   # 85 passed
```

---

# Round R+1 — GpioManagerAdd FP root cause & signature-guarded propagation fix

Date: 2026-08-24 · Build: workspace @ e1f04b8 + local solver/pag changes (uncommitted).
Corpus DBs: baseline `/tmp/opencode/hdf_r4.db`, final `/tmp/opencode/hdf_final.db` (both: `analyze ~/drivers_hdf_core -o <db> --jobs 8 -D __LITEOS__`, unlimited budget via `TRACE_SOLVE_BUDGET_POPS=0`).

## 1. The reported FP and its full derivation

Site: `FinishEvent` — `service->dispatcher->Dispatch(...)` at
`~/drivers_hdf_core/adapter/uhdf2/osal/src/osal_sysevent.c:74`.
Baseline resolved **24 targets; 19 were FPs**, including the suspicious
**`GpioManagerAdd`** (2-param ops callback from gpio_manager.c) plus 10
`*TestEntry`/`*IoDispatch` functions of unrelated subsystems.

Derivation (traced with a temporary solver instrumentation, since removed):

1. **Legit store**: `GpioManagerGet` (`framework/support/platform/src/gpio/gpio_manager.c:90-96`)
   stores `manager->add = GpioManagerAdd` into the `PlatformManager.add` slot.
2. **Wrong-type object flow**: unrelated pointers polluted by casts reach the
   same nodes; the solver's **GEP handler passed any function-kind location in
   the base node's points-to through every field access unchanged** ("fn
   passthrough"), so `fn(GpioManagerAdd)` rode field reads of arbitrary bases.
3. **Memory-content merge**: `merge_memory_into` merged cell *contents* into
   field-*address* nodes, lifting fn locs further; param wiring + casts spread
   them. Pre-fix, 987 distinct nodes held `fn(GpioManagerAdd)`; the static
   dispatcher object cell (`HdfIoDispatcher` object, loc#977) had it written by
   stray stores through wrong-typed pointer sets.
4. **Positional FieldId collisions** then surfaced it wherever a Dispatch-ish
   field was read off a polluted base.

## 2. The fix (implemented)

All in `crates/trace-analysis/src/solver.rs` (+ one helper in `pag.rs`), guarded by declared types only:

- **Store-time cell guard**: a fn value is written into `memory_pts[cell]`
  (and its summary cell) only if the cell accepts it:
  `FnPtr{params}` → same arity; concrete non-fn-pointer cells → reject;
  unknown/untyped cells → accept (conservative).
- **`merge_memory_into` guard**: same predicate when lifting contents.
- **GEP fn passthrough restricted**: registered `array_fn_members` table
  members always pass (HDMI pattern); other fn values pass only when the
  destination field slot's declared FnPtr arity matches
  (`Pag::field_slot_arity`, pag.rs).
- Load fn-passthrough kept unchanged.

Documented in `docs/ANALYSIS.md` (Solver → "Signature-guarded function-value
propagation" + Known imprecision entries).

## 3. Results (baseline → final)

| Metric | Baseline | Final |
|---|---|---|
| Indirect edges | 6494 | **4278** (−2216) |
| Indirect sites with ≥1 target | 1406 | 1390 |
| Arg-flow edges | 16588 | 11983 |

- **FinishEvent** (osal_sysevent.c:74): 24 targets → **exactly**
  `{DeviceManagerDispatch, DeviceNodeExtDispatch, DeviceSvcMgrDispatch,
  HdfKIoServiceDispatch, HdfSyscallAdapterDispatch}`. `GpioManagerAdd`,
  all `*TestEntry`, all `*IoDispatch` gone.
- **HDMI TP preserved**: `HdmiIoDispatch` (`hdmi_dispatch.c`) set identical to
  baseline, all 15 targets.
- **New precision wins**: `cdev->opsImpl->{open,ioctl,poll,release}` now
  resolve to exactly `HdfVNodeAdapter{Open,Ioctl,Poll,Close}`;
  `task->SendMessage → HdfMessageTaskSendMessage`;
  `devmgrService->PowerStateChange → DevmgrServicePowerStateChange`.

### Audited removals (A/B over all sites)

- 131 sites lost some targets (1782 edges). Sampled top losses are all
  cross-family soup (e.g. `opsCall->SetOption` → `GpioDevSetDir`,
  `DevSvcManagerAddService`, Dispatch-family...).
- 22 sites went to zero — investigated individually; **all are stub-side
  calls behind the unmodeled IPC boundary**, e.g.
  `DevSvcManagerStubAddService :: super->AddService`
  (`adapter/uhdf2/manager/src/devsvc_manager_stub.c:289`). Their baseline
  "targets" were provably pollution, not resolution:
  - baseline gave `super->GetObject` → `DevSvcManagerAddService` (wrong impl;
    true impl would be `DevSvcManagerGetObject`);
  - the handler object is reached only via
    `HdfRemoteServiceObtain(inst, &dispatcher)` (external summary) — client
    proxies (`hostClnt->remote->dispatcher->Dispatch`, 13 sites) still resolve
    to `DevSvcManagerStubDispatch` correctly, but no in-process fact links a
    proxy remote to the server stub instance;
  - same class for `DevmgrServiceStubDispatch*` (`devmgr_service_stub.c`),
    `Dispatch{Add,Del}Device` (`devhost_service_stub.c`; DelDevice baseline
    had **101** "targets"), and the two PPG sensor sites whose sole baseline
    target was `DevSvcManagerOnServiceDied` for an `Enable` slot.
  These need an explicit IPC/registration summary design decision, not a
  solver relaxation. Listed as known imprecision in docs/ANALYSIS.md.
- No plausible TP was observed to disappear.

## 4. Verification

```bash
cargo build --workspace && cargo test --workspace   # all pass, clippy clean
TRACE_SOLVE_BUDGET_POPS=0 ./target/release/trace analyze ~/drivers_hdf_core \
  -o /tmp/opencode/hdf_final.db --jobs 8 -D __LITEOS__
# A/B scripts over call_edges keyed by (caller,file,line,callee_text)
```

Note: default 200k budget truncates late sites on this corpus; runs above use
`TRACE_SOLVE_BUDGET_POPS=0`. Deterministic either way.
