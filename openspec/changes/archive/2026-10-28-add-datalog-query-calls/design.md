## Context

See proposal.md. Text recall is the template ([[query#Text Recall]]): the IR leaf `TextPattern` lowers to the `tm_text` table function on SQLite and to a native operator in `tm-native`, both calling one logical operation. Front ends depend only on `tm-ir` ([[architecture#Crates]]); `tm-exec` must not depend on `tm-datalog`, which itself does not depend on `exec` (DL2). The LFTJ call shows how a table-function result is materialised once in an unflattened derived table (`LIMIT -1`) ([[query#Physical Planning#LFTJ]]).

## Goals / Non-Goals

**Goals:** one evaluation per query; identical rows on both routes and in Rust; no new dependency edge from the query engine to Datalog.

**Non-Goals:** correlation, virtual derived predicates, cost-based placement of the call.

## Constraints

- [[architecture#Crates]] constrains layering {strength: must}: front ends see the IR only; the engine sees a trait.
- [[constraints#Same Logical Database]] constrains route equality {strength: must}.
- [[query#Query Budgets]] constrains the call {strength: must}.

## Decisions

### DL17a. Provider trait registered by the facade

`tm-exec` defines `trait DatalogProvider { fn goal(&self, store: &dyn ReadStore, view: &ViewDesc, program: &DlProgramRef, goal: &str) -> Result<DlRows>; }` and holds an optional `Arc<dyn DatalogProvider>` on the `Engine`. The facade registers its implementation (a thin wrapper over `View::datalog`) when built with `exec` and `datalog`. Without it, the leaf fails with `Unsupported` at execution.

- motivated_by:: [[datalog#Own Kernel Over ReadStore]] {weight: primary}

*Alternatives:* `tm-exec` depending on `tm-datalog` behind a feature (couples the engine to one reasoner and breaks the DL2 independence in reverse).

### DL17b. IR leaf and lowering

`DatalogPattern { program: DlProgramRef (Iri | Text), goal: String, outputs: Vec<(String, Var)>, valid: Option<(Var, Var)> }`. SPARQL lowering recognises the service IRI and its property triples on one blank subject; Cypher lowering recognises the procedure name. Goal variables map by name; validation happens in lowering (`DatalogCallShape`), with the goal parsed by the provider at execution.

### DL17c. Once per query

On SQLite the leaf compiles to `(SELECT * FROM tm_datalog(?spec) LIMIT -1)` as an unflattened derived table, which SQLite materialises once; on redb the native runner evaluates the leaf before the join it belongs to and treats its rows as a relation. Explain prints the call once. A planner rule never pushes outer bindings into the leaf.

### Provenance

The provenance pass adds `ProvenanceGap::DatalogCall` when a leaf is present.

## Risks / Trade-offs

- [Users expect correlation] → `DatalogCallShape` names constants as the rule; a recipe shows passing outer values by running the goal with demand on the constants first.
- [Large goal outputs inside queries] → the byte budget counts derived facts; explain shows the row count.

## Migration Plan

Additive; no format change.
