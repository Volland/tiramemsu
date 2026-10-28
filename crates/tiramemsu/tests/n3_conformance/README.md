# eyeron conformance subset

Examples and their expected output (`output/`) copied from
[eyeron](https://github.com/eyereasoner/eyeron) at commit `c4adb8c`
(`examples/` and `examples/output/`), MIT license, copyright Jos De Roo,
KNoWS office of IDLab, Ghent University - imec (see `LICENSE-eyeron.md`).

`tests/n3_conformance.rs` (OpenSpec change `add-n3-reasoning`, task 6.1) loads
each example's facts into a store as statements, passes its rules as N3 text
(printed back from the parsed rules, so the facts never reach the reasoner as
text), runs `View::reason` and compares the conclusions with the expected
output minus the example's own facts, value for value (`5` and `"05"^^xsd:integer`
are the same value). It runs on SQLite, and on redb with the `redb` feature.

Of the 311 examples with an N3 golden at that commit, 144 pass this pipeline;
the 56 here were chosen to cover forward and backward rules, recursion, RDFS
and OWL rule sets, negation as failure (`log:notIncludes`), `log:forAllIn`,
`log:collectAllIn`, queries (`log:query`), inference fuses, and the `math:`,
`string:`, `list:` and `time:` builtins.

Left out, by reason:

- **Facts the store cannot hold** (76 examples): lists, quoted formulas,
  blank nodes or literal subjects among the facts. Statements hold IRIs,
  nodes, statements and literals only.
- **Expected output the store cannot hold** (54): conclusions with lists,
  formulas or blank nodes (existential witnesses are covered by the
  `tm-n3` unit tests and the facade scenario tests instead).
- **Root existential or universal quantification over facts** (34): `@forSome`,
  `@forAll` and the like, which make facts into axioms rather than data.
- **The clock** (`age`): `time:localTime` reads the run's pinned clock, never the
  wall clock, so a golden made on another day differs by design.
- **I/O** (`log:semantics`, `log:content`, `log:parsedAsN3`): rules never read
  files or the network.
- **Run time**: `transitive-closure` and the larger `deep-taxonomy-*` runs pass
  but take seconds; `deep-taxonomy-100` stands for them.
