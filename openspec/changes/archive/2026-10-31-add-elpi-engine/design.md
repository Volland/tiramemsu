## Context

lean-elpi 0.4 adds `leanelpi_declare` (a host predicate from signature text) and `leanelpi_solve_host` (a solve that calls a host function for each declared predicate). It also returns each binding as a JSON term tree. The Rust crate exposes these as `Program::declare`, `Program::solve_host` and `Solution::terms`.

## Goals / Non-Goals

**Goals:** a program reads exactly the view it is given; support cannot be forged; the engine crate has no write path.

**Non-Goals:** lazy row streaming per callback (0.4 returns all rows of one call at once), parallel runs.

## Decisions

### The prelude

Every program is loaded after a fixed prelude:

```
kind term type.
type iri string -> term.        % an IRI
type str string -> term.        % xsd:string
type num int -> term.           % xsd:integer within 63 bits
type dbl float -> term.         % xsd:double
type bool string -> term.       % "true" / "false"
type lang string -> string -> term.   % lexical form, language tag
type lit string -> string -> term.    % lexical form, datatype IRI
type node int -> term.          % an anonymous node
type bnode int -> term.         % a blank node
type tx int -> term.            % a transaction
kind stmt type.
type eid int -> stmt.           % a statement eid
type stmtv stmt -> term.        % a statement as a term
kind bound type.
type at int -> bound.           % epoch milliseconds
type inf bound.                 % unbounded
kind fact type.
type triple term -> term -> term -> fact.
kind severity type.
type error severity.
type warn severity.
pred tm.triple o:term, o:term, o:term, o:stmt.
pred tm.added o:term, o:term, o:term, o:stmt.
pred tm.retracted o:term, o:term, o:term, o:stmt.
pred tm.valid o:stmt, o:bound, o:bound.
pred tm.conclude o:fact, o:list stmt.
pred tm.violation o:severity, o:string, o:string, o:list stmt.
pred tm.violation-delta o:string, o:string, o:list stmt.
```

E8 named native values. ELPI is typed, so each native value sits under a one-argument constructor (`num`, `dbl`, `str`): `(num N)` matches an integer, and `N` is then an ELPI `int`. This refines E8.

### Builtins

`tm.triple S P O E` decodes each bound argument to a value, then to a dictionary id. A value with no id has no rows. The builtin scans the view's statements with the bound positions. With `E` bound it reads that one statement. Rows are decoded with a term cache. The run counts rows against `max_rows`; past the limit the builtin fails with an error, which surfaces as `ElpiOutOfFuel` (the run's budget). `tm.added` and `tm.retracted` read the seeds a delta check is given, and outside a delta check they have no rows. `tm.valid E From To` reads the statement's interval. Every eid a builtin returns goes into the run's seen set.

### Valid time

`query` and `check` take `valid_at`: the view's own instant if it has one, else the clock. `tm.triple` then returns only statements valid at that instant. `conclude` and the constraint runs read every statement of the view whatever its valid time. A conclusion holds over the intersection of its support's intervals, and one whose intersection is empty is dropped. For a Horn program this is the same pointwise result as N5's sweep. A program the scan marks `non_monotonic` is evaluated per segment: the segments are cut at the interval bounds of every eid the first run saw, the run repeats with `tm.triple` limited to the statements valid throughout each segment, and equal neighbours are merged.

### Support

`tm.conclude T S`: `S` must be non-empty, and every eid in it must be in the seen set. Otherwise the run fails with `ElpiRun` (`support not read by this run`, `empty support`). The facade's materialization adds the program's source statement and its included sources later (change 3).

### Monotonicity scan

A program is `non_monotonic` when its text, with comments and strings stripped, contains `not`, `\+`, `!`, `std.findall`, `findall` or `tm.retracted`. The scan is purely textual, so it can only err on the safe side.

## Risks / Trade-offs

- [Callbacks return every row of a call at once] → `max_rows` bounds the memory a run can take. Lazy rows can come in a later lean-elpi release.
- [lean-elpi is about 2.4× slower than ELPI 1.18] → no speed gate is set. The benchmark reports the cost.
