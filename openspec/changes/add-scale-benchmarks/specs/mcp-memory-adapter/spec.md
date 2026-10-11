## ADDED Requirements

### Requirement: Read-only metrics tool
The MCP server SHALL provide a tool `metrics` that takes no arguments, declares `additionalProperties: false` and `readOnlyHint: true`, is listed and callable in read-only mode, and returns the `Db::metrics()` snapshot as JSON. The tool SHALL return no statement, term or query content.

#### Scenario: Listed in read-only mode
- **WHEN** the server runs with `--read-only` and a client calls `tools/list`
- **THEN** `metrics` is listed and calling it returns the snapshot

#### Scenario: Arguments rejected
- **WHEN** `metrics` is called with an argument
- **THEN** the call fails with a structured invalid-arguments error and nothing is read
