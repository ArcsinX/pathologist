# Preprocessor specification

trace implements its own C preprocessor in `trace-preproc`. It runs before tree-sitter parsing so `#include` and `#define` are resolved without invoking gcc/clang.

## API

```rust
pub fn preprocess_file(path: &Path, opts: &PreprocessOptions) -> Result<PreprocessResult>;

pub struct PreprocessResult {
    pub output: String,
    pub line_map: LineMap,
    pub diagnostics: Vec<Diagnostic>,
}
```

CLI equivalents: `--include PATH`, `-D NAME=VALUE`.

## Role in the pipeline

```mermaid
flowchart LR
  Raw[Raw .c on disk]
  IG[IncludeGraph]
  PP[preprocess_file]
  Out[Expanded source]
  TS[tree-sitter parse]

  Raw --> IG
  IG --> PP --> Out --> TS
```

- **`IncludeGraph`** (`trace-parse/src/deps.rs`) scans project files for `#include` directives, builds dependency edges, discovers include directories, and marks which files need preprocessing.
- Preprocessed output is **cached** per file (parallel cache fill when `--jobs > 1`).
- If preprocessing fails hard, parse may fall back to reading raw source (diagnostic recorded).

## Phases

### P0 (implemented)

| Feature | Notes |
|---------|-------|
| Comments | `//`, `/* */` |
| Literals | String and character literals preserved |
| `#include "..."` / `<...>` | Include path stack + `--include` |
| `#define` | Object-like and **function-like** (non-variadic) |
| `##` token pasting | In macro bodies after argument substitution |
| Conditionals | `#ifdef`, `#ifndef`, `#if` / `#elif` (macro-expanded), `#else`, `#endif` |
| `#line` | Location tracking in `LineMap` |
| `#undef` | |
| Predefined | `__FILE__`, `__LINE__` |
| CLI defines | `-D NAME=VALUE` |

### P1 (planned)

- Full `#if` integer constant expression evaluation
- `#pragma once` / include-guard detection
- Variadic macros (`...`, `__VA_ARGS__`)
- `#` stringize operator

### P2 (planned)

- `_Pragma`, additional standard predefined macros

## LineMap

The preprocessor records mappings from **output byte offsets** to original `(file, line, col)` in `LineMap`.

**Current behavior:** tree-sitter parses **preprocessed** source; IR spans (`Span` in `trace-ir`) are resolved through the `LineMap` to original `(file, line, col)` — `#include`d code attributes to its header, TU-local code keeps its original pre-expansion position, and macro-expanded code attributes to the expansion site's origin (identical coordinates when nothing was expanded). Cached `#include` expansions store their own sub-`LineMap`, which is spliced back on replay so origins survive caching.

The `LineMap` must keep byte-accurate offset mapping when extending the preprocessor.

## Include resolution

For `#include "header.h"` / `#include <header.h>`:

1. Directory of the including file
2. Paths from `IncludeGraph.include_dirs` (discovered + `--include`)
3. Error diagnostic if not found

Only **project-local** files under the analysis root are linked; system headers outside the tree are not resolved unless present in the project.

## Include graph and header indexing

| Behavior | Notes |
|----------|-------|
| `needs_preprocess` set | Files with `#include` edges (or included by another) run through the preprocessor |
| `source_cache` | Reuse file text while scanning `#include` edges |
| Reachable headers | Headers transitively `#include`d from any `.c` are expanded into that TU; not indexed as separate units |
| Orphan headers | Project `.h` never reached from any `.c` are indexed as their own units (may contain calls) |
| Parallel index | Orphan headers and `.c` TUs: parallel parse/lower, sequential merge |

### Determinism

Indexing output must be identical across runs of the same tree. Two mechanisms guarantee this:

- **Macro warm pass** runs sequentially over C-reachable headers in canonical (`index_order`) order. Each header is warmed under a **fresh macro table** seeded only from command-line defines; the per-header final states are merged into a union table handed to later phases. Sharing one accumulating table across headers let include guards defined by earlier-warmed headers starve later headers' expansions (the starved text was then frozen into the expansion cache). Dedup between headers comes from the shared expansion cache, not from shared guard state.
- **Expansion-cache freeze**: during parallel phases the include-expansion cache is read-only (`PreprocessOptions::frozen_expansion_cache`). Hits replay warm-pass entries (produced deterministically); misses expand inline under each TU's own macro/guard state and are *not* inserted — first-writer-wins inserts would make results scheduling-dependent.

Translation units inherit the **union** of all warm-pass macro states: cached expansions replay without executing their `#define` directives, so TU-local code still needs those macros.

### Cache self-containment

Cached expansions are flat text — nested `#include`s inside an entry were already resolved when the entry was built. An entry built while a nested header was guard-skipped would otherwise freeze *without* that header's content, permanently hiding its definitions from every consumer routed through the entry. Therefore, while entries are being constructed (non-frozen phases), a guard-skip whose file has a cached expansion **re-splices** the cached text (`splice_cached`): every entry is self-contained. In frozen phases nothing is cached and every replayed entry is already self-contained, so guard-skips stay silent there — splicing would only duplicate parse/lower work (measured +48% index time). Duplicate definitions this introduces are harmless downstream (merge deduplicates same-origin entities; re-declarations remain valid C).

Entries also record which files they claim (`IncludeExpansion.files`); files whose expansion emitted nothing are not claimed, so symbol-scope registration (`headers_of`) does not attribute phantom contributions. A cached-header include whose entire body was skipped emits a visible Warning during non-frozen phases ("resolved include expanded to nothing") — silence here is how starvation bugs historically went unnoticed.

`index_order` itself is canonical: input files are sorted and dependents are visited in sorted order, so unordered `HashSet`/`HashMap` iteration cannot leak into processing order.

### Include-dir self-sufficiency

Project headers must resolve **without manual `-I` flags**: `discover_include_dirs` adds the root, every discovered header's parent directory, and every directory named `include`; a unique-basename fallback resolves names that match exactly one project file. Analyzing a tree root (e.g. an entire source checkout) therefore needs no include-path configuration.

Manual `-I` remains appropriate only for things the tool cannot discover:

- headers **outside** the analyzed root (system SDKs, vendored deps, sibling trees) — when analyzing a subdirectory whose dependencies live elsewhere;
- **platform selection**: when several dirs contain same-basename twins (e.g. per-OS adapter layers), `-I` order picks the intended one — discovery order is sorted-path and not platform-aware;
- paired with `-D` for the matching platform macros (e.g. `-D __LITEOS__`).

**Limitation:** Reachability is computed from literal `#include` lines in raw source (no macro expansion). Headers included only via macros may be misclassified as orphan (duplicate work, usually still correct). Headers excluded by `#if 0` in the preprocessor but visible in the raw graph are treated as reachable and not indexed separately — if the TU also omits them at preprocess time, calls in those headers can be missed.

## Error recovery

| Condition | Behavior |
|-----------|----------|
| Unknown `#directive` | Warning, skip line |
| Missing include | Error on TU |
| Unterminated `#if` | Error at EOF |
| Macro-argument parse failure | Warning `preprocess stopped in <file>`; output produced so far is kept |
| Preprocess failure (hard error) | Diagnostic; unit falls back to raw read |

A mid-run stop inside ONE nested header must not invalidate the whole TU: indexing keeps the truncated-but-LineMap-consistent prefix rather than falling back to raw source, because raw text drops every `#include`d declaration and feeds the parser unexpanded function-like macros. The stop message names the file where processing stopped so downstream tools can report the truncation point.

## Unsupported (v1)

- `_Pragma`
- `#import` (Objective-C)
- `#warning` / `#error` (partially recognized)
- Full C11 macro prescan/rescan semantics
- System include paths outside project tree (unless copied into tree)

## Testing

- Unit tests: `trace-preproc/src/`
- Integration fixtures: `tests/fixtures/preproc/`

See [ARCHITECTURE.md](ARCHITECTURE.md) for how preprocessing fits the full workflow.
