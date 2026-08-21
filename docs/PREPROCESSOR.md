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

**Current behavior:** tree-sitter parses **preprocessed** source; IR spans (`Span` in `trace-ir`) use line/column from that parse on the TU file path. Exported SQLite line numbers therefore reflect **preprocessed** coordinates (includes/macros expanded), not always the original on-disk line.

The `LineMap` is available for future remapping to original locations. Do not break offset mapping when extending the preprocessor.

## Include resolution

For `#include "header.h"` / `#include <header.h>`:

1. Directory of the including file
2. Paths from `IncludeGraph.include_dirs` (discovered + `--include`)
3. Error diagnostic if not found

Only **project-local** files under the analysis root are linked; system headers outside the tree are not resolved unless present in the project.

## Include graph optimizations

| Optimization | Purpose |
|--------------|---------|
| `needs_preprocess` set | Skip preprocess for TUs with no includes/defines |
| `source_cache` | Reuse file text while scanning `#include` edges |
| Orphan header skip | Headers never reached from any `.c` are not indexed separately |
| Parallel precompute | `--jobs` fills preprocess cache before TU indexing |

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
