# SQLite schema

Schema version: **v1**

See also the [README](../README.md) for CLI flags that control what is exported.

## Export modes vs tables

| Table | Minimal (default) | `--full-export` | `--debug-points-to` |
|-------|-------------------|-----------------|---------------------|
| `analysis_run` | ✓ | ✓ | ✓ |
| `files` | ✓ | ✓ | ✓ |
| `functions` | ✓ | ✓ | ✓ |
| `call_sites` | filtered | filtered | filtered |
| `call_edges` | ✓ | ✓ | ✓ |
| `arg_flow_edges` | ✓ | ✓ | ✓ |
| `variables` | arg-flow only | all | all (+ arg-flow) |
| `types` | | ✓ | ✓ |
| `locations` | | ✓ | ✓ |
| `points_to` | | | ✓ |
| `diagnostics` | ✓ | ✓ | ✓ |

### Call site export filter

A row is written to `call_sites` when **any** of:

- the site has ≥1 row in `call_edges`
- the site has ≥1 row in `arg_flow_edges`
- `is_direct = 0` (indirect / fn-ptr syntax, **including unresolved**)

Unresolved indirect calls therefore appear in `call_sites` with zero `call_edges`.

## Entity relationships

```text
analysis_run
files ─┬─ functions ─┬─ call_sites ─┬─ call_edges → functions (callee)
       │             │              └─ arg_flow_edges → variables
       └─ variables (type_id → types when exported)
types
locations (full export)
points_to (debug)
diagnostics
```

## Tables

### analysis_run

| Column | Type | Description |
|--------|------|-------------|
| `id` | INTEGER PK | Run id |
| `trace_version` | TEXT | trace version string |
| `target_root` | TEXT | Analyzed directory |
| `created_at` | TEXT | Unix timestamp (seconds) |
| `options_json` | TEXT | JSON: `include_paths`, `defines`, `include_points_to`, `full_detail` |

### files

| Column | Type | Description |
|--------|------|-------------|
| `id` | INTEGER PK | File id |
| `path` | TEXT UNIQUE | Absolute/normalized path |
| `sha256` | TEXT | Hash placeholder (may be empty) |

### functions

| Column | Type | Description |
|--------|------|-------------|
| `id` | INTEGER PK | Function id |
| `name` | TEXT | Linkage-visible name |
| `file_id` | INTEGER FK → `files` | Primary file |
| `line_start` | INTEGER | Start line (preprocessed) |
| `line_end` | INTEGER | End line (currently ≈ start) |
| `linkage` | TEXT | `external`, `internal`, `none` |
| `signature` | TEXT | Placeholder `fn_<name>` |

**Index:** `functions(name)`

### call_sites

| Column | Type | Description |
|--------|------|-------------|
| `id` | INTEGER PK | Call site id |
| `caller_fn_id` | INTEGER FK → `functions` | Containing function |
| `file_id` | INTEGER FK → `files` | Call location file |
| `line` | INTEGER | Line (preprocessed) |
| `col` | INTEGER | Column |
| `callee_text` | TEXT | Surface syntax (`foo`, `p->handler`, …) |
| `is_direct` | INTEGER | `1` direct by name; `0` indirect |

### call_edges

| Column | Type | Description |
|--------|------|-------------|
| `id` | INTEGER PK | Edge id |
| `call_site_id` | INTEGER FK → `call_sites` | Call site |
| `callee_fn_id` | INTEGER FK → `functions` | Resolved target |
| `resolution` | TEXT | `direct`, `indirect`, `ambiguous` |

Multiple rows per call site are allowed (may-analysis indirect targets).

**Indexes:** `call_edges(callee_fn_id)`, `call_edges(call_site_id)`

### arg_flow_edges

| Column | Type | Description |
|--------|------|-------------|
| `id` | INTEGER PK | Edge id |
| `call_site_id` | INTEGER FK → `call_sites` | Call site |
| `arg_index` | INTEGER | 0-based argument index |
| `actual_var_id` | INTEGER FK → `variables` | Actual variable (`NULL` if actual is a function) |
| `actual_fn_id` | INTEGER FK → `functions` | Actual function for fn-ptr args (`NULL` if actual is a variable) |
| `formal_var_id` | INTEGER FK → `variables` | Callee parameter var |

Exactly one of `actual_var_id` or `actual_fn_id` is set per row.

**Index:** `arg_flow_edges(call_site_id)`

### variables

| Column | Type | Description |
|--------|------|-------------|
| `id` | INTEGER PK | Variable id |
| `name` | TEXT | Source or synthetic name |
| `kind` | TEXT | `global`, `file_static`, `fn_static`, `param`, `local` |
| `fn_id` | INTEGER FK → `functions` | Enclosing function (nullable) |
| `type_id` | INTEGER FK → `types` | Type id |
| `file_id` | INTEGER FK → `files` | Declaration file |
| `line` | INTEGER | Declaration line |

### types

Exported with `--full-export` only.

| Column | Type | Description |
|--------|------|-------------|
| `id` | INTEGER PK | Type id |
| `kind` | TEXT | `void`, `int`, `struct`, `ptr`, … |
| `name` | TEXT | Display name |
| `size` | INTEGER | Layout size |
| `layout_json` | TEXT | JSON field layout |

### locations

PAG abstract locations (`--full-export`).

| Column | Type | Description |
|--------|------|-------------|
| `id` | INTEGER PK | Location id |
| `kind` | TEXT | `Global`, `Local`, `FieldSummary`, `Function`, … |
| `desc` | TEXT | Description |
| `type_id` | INTEGER FK → `types` | Optional |

### points_to

PAG node → location sets (`--debug-points-to`).

| Column | Type | Description |
|--------|------|-------------|
| `var_node_id` | INTEGER | PAG node id |
| `loc_id` | INTEGER FK → `locations` | Target location |

PK: `(var_node_id, loc_id)`

### diagnostics

| Column | Type | Description |
|--------|------|-------------|
| `id` | INTEGER PK | Diagnostic id |
| `severity` | TEXT | `error`, `warning`, `info` |
| `file_id` | INTEGER FK → `files` | Optional |
| `line` | INTEGER | Line |
| `message` | TEXT | Text |
| `stage` | TEXT | `preprocess`, `parse`, `analysis` |

## Example queries

### Callees of a function

```sql
SELECT callee.name, ce.resolution, cs.line, cs.callee_text
FROM call_edges ce
JOIN call_sites cs ON cs.id = ce.call_site_id
JOIN functions caller ON caller.id = cs.caller_fn_id
JOIN functions callee ON callee.id = ce.callee_fn_id
WHERE caller.name = 'HdfSbufReadBuffer';
```

### Unresolved indirect call sites

```sql
SELECT caller.name, cs.line, cs.callee_text
FROM call_sites cs
JOIN functions caller ON caller.id = cs.caller_fn_id
LEFT JOIN call_edges ce ON ce.call_site_id = cs.id
WHERE cs.is_direct = 0 AND ce.id IS NULL
ORDER BY caller.name, cs.line;
```

### Indirect calls only (resolved)

```sql
SELECT caller.name, callee.name, cs.callee_text, cs.line
FROM call_edges ce
JOIN call_sites cs ON cs.id = ce.call_site_id
JOIN functions caller ON caller.id = cs.caller_fn_id
JOIN functions callee ON callee.id = ce.callee_fn_id
WHERE ce.resolution = 'indirect';
```

### Callers of a function

```sql
SELECT caller.name, ce.resolution, cs.line
FROM call_edges ce
JOIN call_sites cs ON cs.id = ce.call_site_id
JOIN functions caller ON caller.id = cs.caller_fn_id
JOIN functions callee ON callee.id = ce.callee_fn_id
WHERE callee.name = 'LiteNetSetIpAddr';
```

### Argument flow at a call site (variable actuals)

```sql
SELECT cs.line, af.arg_index, av.name AS actual, fv.name AS formal
FROM arg_flow_edges af
JOIN call_sites cs ON cs.id = af.call_site_id
JOIN variables av ON av.id = af.actual_var_id
JOIN variables fv ON fv.id = af.formal_var_id
WHERE af.actual_var_id IS NOT NULL;
```

### Argument flow (function-pointer actuals)

```sql
SELECT cs.line, af.arg_index, f.name AS actual_fn, fv.name AS formal
FROM arg_flow_edges af
JOIN call_sites cs ON cs.id = af.call_site_id
JOIN functions f ON f.id = af.actual_fn_id
JOIN variables fv ON fv.id = af.formal_var_id
WHERE af.actual_fn_id IS NOT NULL;
```

## CLI inspection

```bash
trace inspect graph.db calls [--from FN] [--to FN]
```

Lists rows from `call_edges` joined with `call_sites` / `functions`. Unresolved indirect sites require SQL (query above).
