# Tool rework evaluation

This document fixes the comparison protocol for task 14. It does not claim a
model benchmark was run: this checkout has no configured 3B–10B model endpoint.

## Fixed baseline

The baseline is commit `850c2d7`, before the tool rework. The comparison keeps
the same user intent and changes only the tool API:

| Baseline API | New API | Main contract change |
| --- | --- | --- |
| `run_in_terminal(code, timeout_milliseconds)` | `run_command(command)` | server timeout and bounded result |
| `fetch_webpage(url, timeout_milliseconds)` | `fetch_url(url)` | HTTP(S) only and bounded readable content |
| `move_path(from, to)` | `rename_path(source, destination)` | no merge and no overwrite |
| `delete_file(path)`, `delete_directory(path)` | `delete_path(path)` | one file/directory operation |
| `stat(path)` | removed | type is determined by the specialized operation |

The baseline schemas and descriptions must be captured from that commit before
running a model comparison. New-run records must use the same scenario IDs
below, so schema, description, runtime, and recovery regressions remain
separable.

## Scenarios

1. `find_file_by_name_part` — call `find_paths` with a case-insensitive name part.
2. `find_directory_by_name_part` — call `find_paths` and select a directory result.
3. `search_case_sensitive_text` — call `search_text` with exact case.
4. `read_large_file_in_chunks` — call `read_file` with `path`, then continue from `end_line` when `has_more` is true.
5. `apply_one_replacement` — call `read_file`, then one `apply_patch` with its `revision`.
6. `create_nested_file` — call `create_file` with `path` and `content`; do not create parents first.
7. `create_existing_directory` — call `create_directory` twice and accept `action: unchanged`.
8. `delete_missing_path` — call `delete_path` and accept `action: not_found` without recovery.
9. `delete_file_and_directory` — call `delete_path` for both kinds and confirm the prompt.
10. `rename_file` — call `rename_path` with `source` and `destination`.
11. `rename_existing_destination` — observe `PATH_ALREADY_EXISTS` and choose a new destination.
12. `run_test_command` — call `run_command` with one `command`; inspect exit code and output.
13. `fetch_page` — call `fetch_url` with one HTTP(S) `url`; inspect final URL and status.
14. `recover_revision_changed` — reread the file after `REVISION_CHANGED`.
15. `recover_text_not_found` — reread/search before retrying after `TEXT_NOT_FOUND`.
16. `stop_after_user_refusal` — do not retry after `USER_REFUSED`; return a final answer.

## Matrix and record format

Run every scenario against each available 3B–4B instruct, 7B–8B instruct, and
up-to-10B instruct model, using native tool calling and text-emulated calling
when supported. Store one JSON object per failed attempt:

```json
{
  "scenario": "rename_existing_destination",
  "model": "model-id",
  "mode": "native",
  "tool_call": {"name": "rename_path", "arguments": {}},
  "error_code": "PATH_ALREADY_EXISTS",
  "next_response": "raw assistant response",
  "classification": "schema|description|runtime|recovery"
}
```

For every run, record valid-JSON rate, correct-tool rate, scenario success,
tool turns, recovery turns, repeated identical errors, `run_command` used for a
filesystem operation, schema bytes, and tool-output bytes. Compare each metric
with the fixed baseline; do not pool native and text-emulated results.

## Local verification recorded here

The deterministic local suite covers the new schemas, path safety, action and
collection envelopes, error codes, recovery no-op detection, bounded output,
and server-side timeout states. The model matrix remains pending until a local
or remote model endpoint and its model IDs are supplied.
