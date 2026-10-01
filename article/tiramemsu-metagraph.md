# Half a metagraph, and the other half

*Tiramemsu already lets an edge be a node. This post covers the rest: nodes and edges that hold whole subgraphs, nested to any depth, n-ary edges, and fold and unfold, built from the same one row shape plus tags.*

![A tiramisu slice where one layer is lifted out and turns out to contain a smaller tiramisu](images/mg-01-cover.png)
*Placeholder: cover. A metavertex is a layer that holds a graph of its own.*

---

> **Status.** Every snippet marked *works today* was run against the current build. The one engine change this post asked for, a statement eid as a graph name, landed on 2026-10-01 (decision D36), so an edge can now hold a subgraph.

In the first long read I wrote that Tiramemsu is "a metagraph made practical: edges are first-class, so edges can connect edges." That is true, but it is only half the story, and readers of my book on metagraphs noticed.

A metagraph has two defining powers:

1. **Edges are vertices.** A relationship can be the subject or object of another relationship, to any depth.
2. **Vertices are containers.** A vertex can hold a whole subgraph, and so can an edge, recursively.

Tiramemsu has had the first power since day one. This post is about the second: what already worked if you knew the pattern, and the one small engine change that closed the gap without giving up anything the design stands on.

## What a metagraph is, in one page

The formal version comes from Basu and Blanning: a metagraph `M = ⟨X, E⟩` has a set of elements `X` and a set of metaedges `E`, where a metaedge goes from a *set* of elements to a *set* of elements, and a metaedge can itself be an element. Gapanyuk's annotated metagraph adds that metavertices and metaedges carry annotations that are themselves metagraphs. Goertzel adds two operations I find the most practical of all:

- **Unfold**: expand a metavertex into the subgraph it stands for.
- **Fold**: collapse a subgraph into one metavertex, so you can reason about it as a unit.

In *Metagraph for AI Agents* I describe the problem this solves as the **hypergraph wall**. A hyperedge connects many nodes, but it is a connection, not a thing, so you cannot say anything *about* it. The moment an agent needs "this meeting caused that decision, and I'm 70% sure of it", the relationship has to become something you can point at. You then want to open that thing up and look inside.

An agent's memory runs into both. A conversation is a node that holds the facts said in it. A project holds teams, which hold people and the services they own. An enrollment in a clinical trial is an edge between a patient and a trial, and it has its own investigators, site and consent constraints. None of these is a flat list of triples.

## The half we already have

Every fact in Tiramemsu is a row `(eid, s, p, o)`, and the eid is an ordinary id. Put it in the subject or object position of another statement and you have a layer:

```sparql
INSERT DATA {
  v:alice v:worksAt v:acme {| v:confidence 0.8 ; v:source "chat-2026-09-29" |} .
}
SELECT ?b ?c WHERE { ?b v:supportedBy ?e . ?e v:confidence ?c }
```

Cypher sees the same eid as both a relationship and a node, so a relationship variable can stand where a node goes:

```cypher
MATCH (a)-[r:worksAt]->(c), (b:Belief)-[:supportedBy]->(r)
RETURN a, c, r.confidence, b
```

The rest of the engine already treats this as a graph of graphs. Retracting a fact cascades to everything whose subject or object is that fact, recursively. `View::dependents(e)` lists that closure without retracting anything. The path engine has virtual `sys:subject` and `sys:object` hops, so a recursive walk can climb from a belief down through its evidence. Schema is stored as triples, so the model describes itself.

So edges are vertices. What we don't have yet is containers.

![Diagram: layer 0 facts, layer 1 annotations, layer 2 beliefs, all pointing down by eid](images/mg-02-half.png)
*Placeholder: the half we have. Edges point at edges, layer by layer.*

## One idea: a metavertex is any id that has contents

Here is the whole proposal in one sentence. **A metavertex is any id that has contents, and contents are tags.**

Tiramemsu already has tags. A named graph is a node, and membership is a layer statement on a fact's eid:

```
(e1  sys:inGraph  v:session12)
```

Graphs are tags, not containers. That was a deliberate decision (D28): one statement keeps one eid however many graphs it sits in, so asserting it again stays idempotent, and the confidence on a fact belongs to the fact, not to one copy of it. Removing a membership never retracts the member.

Read in metagraph terms, a named graph is already a metavertex. It is a node that has other statements tagged into it. It can carry its own triples (`v:session12 v:startedBy v:agent7`), and those are ordinary facts about the metavertex.

The rest of this post takes that one idea and pushes it in four directions:

1. Let the id with contents be a **statement**, so an edge can be a container.
2. Let metavertices sit **inside** metavertices.
3. Let an edge connect **sets**, not just pairs.
4. Give names to the two moves, **fold** and **unfold**.

One rule holds throughout. Contents that are statements go through `sys:inGraph`. Contents that are plain vertices go through an ordinary statement such as `(v:teamA v:contains v:alice)`, because nodes in Tiramemsu have no lifetime of their own and exist only while some statement mentions them. Either way, containment is made of statements, and statements have two clocks. That will matter at the end.

## Power one: an edge that holds a subgraph

Take an enrollment. Patient 7 is enrolled in trial 3. That is an edge. The enrollment also has an investigator, a site, a consent version and an eligibility rule, and those belong to *this* enrollment, not to the patient or the trial in general.

The natural model makes the enrollment edge the name of its own subgraph:

```sparql
# works today (since 2026-10-01)
INSERT DATA {
  GRAPH <urn:tiramemsu:stmt:1> {        # the eid of (v:p7 v:enrolledIn v:trial3)
    v:drSmith v:role    v:investigator .
    v:site9   v:hosts   v:p7 .
  }
}
```

Until 2026-10-01 this failed with `InvalidGraphName`, because a graph name had to be an IRI, an anonymous node or a blank node. The workaround was one level of indirection: tag the edge with a context node, and put the contents in that node's graph.

```sparql
INSERT DATA {
  v:p7 v:enrolledIn v:trial3 {| v:context v:enroll1 |} .
  GRAPH v:enroll1 { v:drSmith v:role v:investigator . v:site9 v:hosts v:p7 }
}
```

That is honest modelling, but it is the same move the book warns about: you reify the edge into a stand-in node, and the stand-in can drift from the edge it stands for. Correct the enrollment with `supersede` and the context node doesn't know.

**The lift** was to accept a statement eid as a graph name. It was small, because nothing about membership assumed the graph name is a node:

- `add_to_graph`, SPARQL `GRAPH` blocks, `FROM`, `INSERT DATA`, `WITH`, `CLEAR` and `DROP` accept a `STMT` name. `GRAPH ?e { … }` works with `?e` bound by the reifier syntax `~ ?e`. Adding to a retracted statement fails with `NotLive`.
- **The cascade already does the right thing.** A membership is `(m sys:inGraph e)`, so its object is `e`. Retracting `e` cascades to every live statement whose object is `e`, which retracts the memberships and leaves the members alone. That is exactly "the container is gone, the facts stay" (D8 plus D28).
- **Export already has a name for it.** A statement renders as `urn:tiramemsu:stmt:<n>`, which is a valid graph IRI in N-Quads and parses back to the same id.

The open question was `supersede`. When an edge is corrected, its layers are replayed onto the new eid. Should its contents follow too? Yes: the enrollment's investigator is still the investigator after we fix a typo in the trial id. Supersede now replays every membership whose graph is in the corrected set onto the new eid, and the members keep their ids. A member's own memberships in other graphs are still dropped and left to the writer, as before.

![Diagram: the enrolledIn edge drawn as a box containing the investigator and site statements](images/mg-03-edge-container.png)
*Placeholder: an edge that holds a subgraph.*

## Power two: metavertices inside metavertices

Nesting needs no engine change at all. It is an ordinary statement between two metavertices:

```sparql
# works today
INSERT DATA {
  GRAPH v:teamA { v:alice v:owns v:billing }
  GRAPH v:teamB { v:bob   v:owns v:search  }
  GRAPH v:teamC { v:carol v:owns v:ads     }
  v:teamA    v:within v:payments .
  v:payments v:within v:org .
  v:teamB    v:within v:org .
}
```

"Everything owned anywhere under the org" is a property path that finds the metavertices, and a `GRAPH` block that opens them:

```sparql
# works today
SELECT ?g ?who ?svc WHERE {
  ?g v:within* v:org .
  GRAPH ?g { ?who v:owns ?svc }
}
```

That returns team A (two levels down) and team B (one level down), and not team C, which sits outside. I also added a cycle, `v:org v:within v:payments`, to see what happens. The answer doesn't change, because the path engine keeps a visited set.

Cypher reaches the same data through the dual view. A membership is a relationship from a statement to its graph, and the `sys:` predicate is reachable by its CURIE:

```cypher
// works today
MATCH (a)-[r:owns]->(b), (r)-[:`sys:inGraph`]->(g)-[:within*0..]->(top {`@id`: 'v:org'})
RETURN DISTINCT g, a, b
```

A variable-length Cypher match gives one row per path, so with the cycle above it needs `DISTINCT`.

**An optional lift** is sugar for this: a "deep" graph selector that expands a graph set by the `within*` closure before the membership join. The path engine already filters every hop by a graph set (D33), so a deep selector would also give recursive walks that stay inside an org and all its teams. That is convenience, not capability, and I'd build it only after someone writes the long form a few times.

## Power three: edges between sets

Basu and Blanning's metaedge goes from a set to a set. A hyperedge connects many members at once. Both are the same question: how do you store a relationship with more than two ends in a store whose rows have exactly two?

Tiramemsu's answer, decided early and written down in the design options, is the TypeDB one: **the relationship is a thing, and its ends are role statements on it.** There are two ways to write it, and both work today.

**As a node**, when the relationship is the main thing:

```sparql
# works today
INSERT DATA {
  v:m1  a v:Meeting ; v:participant v:alice, v:bob, v:carol ; v:decided v:d1 .
  v:me1 v:from v:alice, v:bob ; v:to v:ads, v:search .   # a set-to-set metaedge
  v:m1  v:causes v:me1 .                                 # a metaedge between metaedges
}
```

**As a statement**, when there is a natural binary core and the other ends are extra:

```sparql
# works today
INSERT DATA { v:alice v:meets v:bob {| v:with v:carol ; v:with v:dave |} }
SELECT ?a ?b ?x WHERE { ?a v:meets ?b ~ ?e . ?e v:with ?x }
```

```cypher
// works today
MATCH (a)-[r:meets]->(b), (r)-[:with]->(x) RETURN a, b, x
```

The second form has a nice property: the meeting *is* the edge, so it gets the edge's lifetime, valid time and cascade for free. Retract the meeting and its extra participants go with it.

To keep role predicates honest, use typed layers (D30). Declare that `v:with` only annotates statements, and a role written on a plain node by mistake is refused instead of silently becoming a property:

```sparql
# works today
INSERT DATA { v:with sys:subjectType sys:STMT }
INSERT DATA { v:alice v:with v:carol }    # SubjectTypeMismatch
```

The book is frank about the trade-off, and so am I. Reifying a relationship makes it addressable, but you lose the homogeneous "show me every n-way relationship" query that a native hyperedge store gives you, and adding or removing a member is a write on the relationship rather than on a set. I rejected the other classic answer, content-addressed hyperedges where the id is a hash of the type and member set. It dedups beautifully, but it cannot say "the same meeting happened twice", and parallel edges are something an agent's memory needs.

## Power four: fold and unfold

With graphs as tags, both of Goertzel's operations turn out to be plain writes and reads.

**Fold** is a write: mint a metavertex and tag the subgraph into it. From SPARQL, `INSERT … WHERE` with a `GRAPH` template adds memberships to statements that already exist. Because assert is idempotent, it doesn't copy them:

```sparql
# works today
INSERT { GRAPH v:episode1 { ?s v:owns ?o } . v:episode1 v:summarizes "who owns what" }
WHERE  { ?s v:owns ?o }
```

After this there are still exactly three `v:owns` statements, each now a member of its team's graph *and* of `v:episode1`. From Rust it is `tx.add_to_graph(e, &v("episode1"), AssertOpts::default())` per statement, in one transaction.

**Unfold** is a read: open the metavertex.

```sparql
# works today
SELECT ?s ?p ?o WHERE { GRAPH v:episode1 { ?s ?p ?o } }
```

To unfold with the evidence, not just the members, add `View::dependents(e)` per member: its layers, the beliefs that cite it, and its memberships. That is the same closure the cascade would walk.

**Moving a folded metavertex** to another agent is the one place today's tools are too narrow. A fact bundle (`View::bundle(e)`) carries one root statement with its layers, its evidence and its memberships, and imports by assert on the other side. A metavertex has many roots. Today that means one bundle per member. A bundle rooted at a graph, or at a statement graph, is the natural next step. It is the only fold/unfold feature I'd add to the engine rather than leave as a recipe.

![Diagram: a subgraph folded into one vertex, and the same vertex unfolded back](images/mg-04-fold.png)
*Placeholder: fold is a write of tags, unfold is a read of them.*

## Time comes for free

This is the part I like most. Containment, nesting and roles are all statements, so they all have both clocks, and every temporal query already works on the metagraph's structure, not just its facts.

After team B moves out from under the org:

```sparql
DELETE DATA { v:teamB v:within v:org }
```

the deep query now returns only team A. The same query as of the transaction before the move returns both, as a whole query or scoped to one pattern:

```sparql
# works today; 150 stands for the transaction before the move
SELECT ?g WHERE {
  SERVICE <urn:tiramemsu:tm:asOf/150> { ?g v:within* v:org . GRAPH ?g { ?who v:owns ?svc } }
}
```

Nothing was deleted. The nesting statement was retracted, so "which teams sat under the org last Tuesday" has an exact answer, and so does "when did we learn that team B moved". Give the nesting a valid-time interval when you write it (`Valid::between(from, to)` in Rust), and `FROM <urn:tiramemsu:tm:validAt/2024-06-01>` answers "which teams were under the org in June 2024", whenever we happened to record it. A time-respecting walk (D34) over `v:within+` never steps into a nesting that had already ended when the walk got there.

A memory that remembers being wrong about a fact should also remember being wrong about a structure. With containers built from statements, it does.

## What I'm deliberately not doing

Three ideas come up every time metagraphs do, and I'm leaving all three out.

- **An eid as a predicate.** A metagraph in the strictest sense lets an edge be the *type* of another edge. In Tiramemsu the predicate is always an IRI. Allowing a statement there would change the index layout, the planner's statistics and the Cypher/RDF name mapping, all to express something a role statement already says.
- **Containers that own their contents.** If a graph owned its statements, a fact in two graphs would be two facts, and its confidence would split in two. Tags keep one fact, one eid, one confidence (D28).
- **Content-addressed hyperedges.** As above: elegant dedup, but no parallel edges.

## Where it stands, honestly

- **Works today:** nested metavertices through a `within*` path and `GRAPH ?g`, in SPARQL and in Cypher through the dual view. N-ary and set-to-set edges as role statements on a node or a statement, with typed layers to keep them honest. Fold as `INSERT … WHERE` into a graph, and unfold as a `GRAPH` read plus `dependents`. All of it under `asOf`, `validAt` and time-respecting paths.
- **Landed on 2026-10-01:** a statement eid as a graph name (edge as container). The contents go with a retraction of the edge and follow a supersede.
- **Optional:** a deep graph selector, and a bundle rooted at a metavertex.
- **Open questions:** the N-Quads round trip of a statement-named graph (there is no N-Quads export yet), and whether Cypher deserves its own syntax for containment or the dual view is enough. A membership keeps its own valid time rather than the edge's, like any other layer.

## The point

The hard part of a metagraph was never the theory. It is finding a representation where "an edge is a node" and "a node is a graph" don't each need their own table, their own id space and their own query language.

In Tiramemsu both come from the same two things that were already there. A row with an address gives edges that are vertices. A layer that tags that address gives vertices and edges that are containers. There is no new storage, no new column, and nothing to forget.

*The design lives in the repository's `lat.md/` knowledge graph. The metagraph background is in* Metagraph for AI Agents.
