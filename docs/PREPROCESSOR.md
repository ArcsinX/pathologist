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

**Current behavior:** tree-sitter parses **preprocessed** source; IR spans (`Span` in `trace-ir`) are resolved through the `LineMap`: code from `#include`d files is attributed to its original file with original line/column, while TU-local code uses positions on the preprocessed text (identical to the raw file when nothing was expanded). Cached `#include` expansions store their own sub-`LineMap`, which is spliced back on replay so origins survive caching.

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

- **Macro warm pass** runs sequentially over C-reachable headers in canonical (`index_order`) order, building the shared macro table. Parallel workers start from that frozen snapshot and never accumulate into it.
- **Expansion-cache freeze**: during parallel phases the include-expansion cache is read-only (`PreprocessOptions::frozen_expansion_cache`). Hits replay warm-pass entries (produced deterministically); misses expand inline under each TU's own macro/guard state and are *not* inserted — first-writer-wins inserts would make results scheduling-dependent.

`index_order` itself is canonical: input files are sorted and dependents are visited in sorted order, so unordered `HashSet`/`HashMap` iteration cannot leak into processing order.

**Limitation:** Reachability is computed from literal `#include` lines in raw source (no macro expansion). Headers included only via macros may be misclassified as orphan (duplicate work, usually still correct). Headers excluded by `#if 0` in the preprocessor but visible in the raw graph are treated as reachable and not indexed separately — if the TU also omits them at preprocess time, calls in those headers can be missed.

## Error recovery

| Condition | Behavior |
|-----------|----------|
| Unknown `#directive` | Warning, skip line |
| Missing include | Error on TU |
| Unterminated `#if` | Error at EOF |
| Preprocess failure | Diagnostic; may fall back to raw read |

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
