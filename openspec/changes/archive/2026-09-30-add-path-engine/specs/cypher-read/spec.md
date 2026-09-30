## REMOVED Requirements

### Requirement: Variable-length and shortest paths pending the path engine
**Reason**: The path engine (add-path-engine, M3) evaluates variable-length relationships, `shortestPath` and `allShortestPaths`; the interim `Unsupported` failure is replaced.
**Migration**: The `path-lowering` capability specifies Cypher variable-length relationships, path and relationship-list bindings, relationship isomorphism, shortest paths, endpoint binding, temporal scope and the forms that stay `Unsupported` (property maps on variable-length relationships, quantified path patterns, `REPEATABLE ELEMENTS` with a variable-length pattern).
