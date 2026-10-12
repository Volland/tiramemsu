#!/usr/bin/env python3
"""Finite checks for revision 4 of the export-boundary paper.

Mathematical models only, except the optional engine trace (engine-trace.json),
which `cargo test -p tm-core --test export_boundary` writes when TM_EXPORT_TRACE is set.
"""
from ast import literal_eval
from collections import deque
from itertools import permutations, product
from pathlib import Path
import hashlib
import json
import random

ROOT = Path(__file__).resolve().parent
NODE, NODE2, LITERAL = ('N', 'a'), ('N', 'b'), ('L', '1')
P, Q = ('I', 'p'), ('I', 'q')
IN, DEFAULT = ('I', 'reserved:in'), ('I', 'reserved:default')
SUPERSEDES = ('I', 'sys:supersedes')
TIME = {'a': ('I', 'reserved:addedAt'), 'd': ('I', 'reserved:retractedAt'),
        'v1': ('I', 'reserved:validFrom'), 'v2': ('I', 'reserved:validTo')}
RESERVED = {IN, DEFAULT, *TIME.values()}
REIFIES = ('I', 'rdf:reifies')


def ref(i):
    return ('E', i)


def targets(row):
    return {term[1] for term in (row[0], row[2]) if term[0] == 'E'}


# --------------------------------------------------------------------------- kernels

def kernel(rows):
    """Revision 2 algorithm: Kosaraju SCCs, then reverse reachability from obstructions."""
    adjacency = {i: targets(r) & rows.keys() for i, r in rows.items()}
    reverse = {i: set() for i in rows}
    bad = {i for i, r in rows.items() if targets(r) - rows.keys()}
    for i, neighbors in adjacency.items():
        for j in neighbors:
            reverse[j].add(i)
    visited, order = set(), []
    for start in rows:
        if start in visited:
            continue
        stack = [(start, False)]
        while stack:
            i, done = stack.pop()
            if done:
                order.append(i)
            elif i not in visited:
                visited.add(i)
                stack.append((i, True))
                stack.extend((j, False) for j in adjacency[i] if j not in visited)
    visited = set()
    for start in reversed(order):
        if start in visited:
            continue
        component, stack = set(), [start]
        while stack:
            i = stack.pop()
            if i in visited:
                continue
            visited.add(i)
            component.add(i)
            stack.extend(reverse[i] - visited)
        if len(component) > 1 or start in adjacency[start]:
            bad.update(component)
    queue = deque(bad)
    while queue:
        for i in reverse[queue.popleft()]:
            if i not in bad:
                bad.add(i)
                queue.append(i)
    return {i: row for i, row in rows.items() if i not in bad}


def peel(rows):
    """Revision 3 algorithm: least fixpoint of F(X) = {r : refs(r) <= X} by a Kahn peel.

    Returns the kernel and its export order (every row after all its targets)."""
    pending, dependents, queue = {}, {i: [] for i in rows}, deque()
    for i, row in rows.items():
        ts = targets(row)
        if ts - rows.keys():
            continue
        pending[i] = len(ts)
        for j in ts:
            dependents[j].append(i)
        if not ts:
            queue.append(i)
    order = []
    while queue:
        i = queue.popleft()
        order.append(i)
        for j in dependents[i]:
            if j in pending:
                pending[j] -= 1
                if pending[j] == 0:
                    queue.append(j)
    return {i: rows[i] for i in order}, order


def fixpoint_iterates(rows):
    """Literal Kleene iteration F^k(empty), independent of both algorithms."""
    current = set()
    while True:
        nxt = {i for i, r in rows.items() if targets(r) <= current}
        if nxt == current:
            return current
        current = nxt


def oracle_admissible(rows, chosen):
    """Independent closure test and transitive closure matrix."""
    ids = sorted(chosen, key=repr)
    index = {i: a for a, i in enumerate(ids)}
    reach = [[False] * len(ids) for _ in ids]
    for a, i in enumerate(ids):
        for term in (rows[i][0], rows[i][2]):
            if term[0] == 'E':
                if term[1] not in chosen:
                    return False
                reach[a][index[term[1]]] = True
    for k in range(len(ids)):
        for a in range(len(ids)):
            if reach[a][k]:
                for b in range(len(ids)):
                    reach[a][b] |= reach[k][b]
    return not any(reach[i][i] for i in range(len(ids)))


def oracle_greatest(rows):
    ids = list(rows)
    admissible = []
    for mask in range(1 << len(ids)):
        chosen = {i for bit, i in enumerate(ids) if mask & (1 << bit)}
        if oracle_admissible(rows, chosen):
            admissible.append(chosen)
    greatest = set().union(*admissible)
    assert oracle_admissible(rows, greatest)
    return greatest


def closed(rows):
    return all(targets(r) <= rows.keys() for r in rows.values())


def reference_closure(selected, store):
    """Least closed superset of `selected` inside `store` (the dual construction C)."""
    out, stack = set(), list(selected)
    while stack:
        i = stack.pop()
        if i in out:
            continue
        if i not in store:
            return None
        out.add(i)
        stack.extend(targets(store[i]))
    return out


# --------------------------------------------------------------------------- encodings

def term_map(terms):
    mapping = {term: term if term[0] in ('B', 'L') else
               ('I', 'encoded:' + repr(term)) for term in terms}
    assert len(set(mapping.values())) == len(mapping)
    assert not (RESERVED & set(mapping.values()))
    return mapping


def encode(rows):
    terms = {term for row in rows.values() for term in row if term[0] != 'E'}
    mapping = term_map(terms)
    graph = {}
    for i, row in rows.items():
        graph[ref(i)] = tuple(term if term[0] == 'E' else mapping[term] for term in row)
        graph[ref(('membership', i))] = (ref(i), IN, DEFAULT)
    leaves = {term for row in graph.values() for term in row if term[0] != 'E'}
    return graph, leaves, {value: term for term, value in mapping.items()}


def decode(graph, inverse):
    return {vertex[1]: tuple(term if term[0] == 'E' else inverse[term] for term in row)
            for vertex, row in graph.items() if row[1] != IN}


def encode_temporal(records, asserted):
    """Full records (s, p, o, a, d, v1, v2) -> role graph with reserved time statements.

    Only records in `asserted` receive a membership; the others are quoted only."""
    terms = {term for r in records.values() for term in r[:3] if term[0] != 'E'}
    mapping = term_map(terms)
    graph = {}
    for i, (s, p, o, a, d, v1, v2) in records.items():
        graph[ref(i)] = tuple(t if t[0] == 'E' else mapping[t] for t in (s, p, o))
        for key, value in (('a', a), ('d', d), ('v1', v1), ('v2', v2)):
            if value is not None:
                graph[ref(('time', key, i))] = (ref(i), TIME[key], ('L', str(value)))
                graph[ref(('membership', 'time', key, i))] = (ref(('time', key, i)), IN, DEFAULT)
        if i in asserted:
            graph[ref(('membership', i))] = (ref(i), IN, DEFAULT)
    leaves = {term for row in graph.values() for term in row if term[0] != 'E'}
    return graph, leaves, {value: term for term, value in mapping.items()}


def decode_temporal(graph, inverse):
    content, clocks, asserted = {}, {}, set()
    for vertex, (s, p, o) in graph.items():
        if p == IN:
            if len(vertex[1]) == 2:
                asserted.add(vertex[1][1])
        elif p in TIME.values():
            key = next(k for k, v in TIME.items() if v == p)
            clocks.setdefault(s[1], {})[key] = int(o[1])
        else:
            content[vertex[1]] = tuple(t if t[0] == 'E' else inverse[t] for t in (s, p, o))
    records = {i: content[i] + tuple(clocks.get(i, {}).get(k) for k in ('a', 'd', 'v1', 'v2'))
               for i in content}
    return records, asserted


def image_valid(graph, leaves, rdf_star=False):
    """Statement Graph axioms plus the RDF-star image (uniqueness optional, Gelling et al. 5.1)."""
    incoming = {vertex: 0 for vertex in graph.keys() | leaves}
    if graph.keys() & leaves:
        return False
    for vertex, row in graph.items():
        if vertex[0] != 'E' or len(row) != 3:
            return False
        s, p, o = row
        if s[0] not in ('E', 'I', 'B') or p[0] != 'I' or o[0] not in ('E', 'I', 'B', 'L'):
            return False
        for term in row:
            if term not in incoming or (term[0] == 'E') != (term in graph):
                return False
            incoming[term] += 1
    if any(incoming[leaf] == 0 for leaf in leaves):
        return False
    for vertex, (s, p, o) in graph.items():
        if p == IN:
            if s[0] != 'E' or o[0] not in ('I', 'B') or incoming[vertex]:
                return False
        elif not incoming[vertex]:
            return False
    grey, black = set(), set()

    def visit(vertex):
        if vertex in grey:
            return False
        if vertex in black:
            return True
        grey.add(vertex)
        for term in graph[vertex]:
            if term in graph and not visit(term):
                return False
        grey.remove(vertex)
        black.add(vertex)
        return True
    if not all(visit(vertex) for vertex in graph):
        return False
    return not rdf_star or len(set(graph.values())) == len(graph)


def identity_preserving(rows, graph):
    """Definition: internal vertices are exactly the records; eid endpoints hit internal vertices."""
    internal = {v[1] for v, row in graph.items() if row[1] != IN}
    return internal == rows.keys() and all(
        graph[ref(i)][k] == term and term in graph
        for i, row in rows.items() for k, term in enumerate(row) if term[0] == 'E')


# --------------------------------------------------------------------------- query view

def denote(graph, vertex):
    """RDF-star reading of a statement vertex: nested quoted triples, leaves as terms."""
    return tuple(('Q', denote(graph, t)) if t[0] == 'E' else t for t in graph[vertex])


def asserted_triples(graph):
    """The RDF-star default graph: triples denoted by vertices that carry a membership."""
    return {denote(graph, s) for (s, p, o) in graph.values()
            if p == IN and graph[s][1] not in RESERVED}


def quoting(rows, mapping):
    """q: eids -> nested quoted triples, terms -> f(term). Well defined on closed, acyclic rows."""
    q = {}

    def term(t):
        return eid(t[1]) if t[0] == 'E' else mapping[t]

    def eid(i):
        if i not in q:
            s, p, o = rows[i]
            q[i] = ('Q', (term(s), mapping[p], term(o)))
        return q[i]
    for i in rows:
        eid(i)
    return q, term


QUERIES = (
    # (?x ?p ?y ~ ?e)
    ((('?x', '?p', '?y', '?e'),)),
    # (?x P ?y ~ ?e) . (?e Q ?z)
    ((('?x', P, '?y', '?e'), ('?e', Q, '?z', None))),
    # (?e ?p ?y ~ ?f) . (?x P ?y2 ~ ?e)
    ((('?e', '?p', '?y', '?f'), ('?x', P, '?y2', '?e'))),
    # (?e Q ?z) . (?e P ?w): two annotations joined on one statement
    ((('?e', Q, '?z', None), ('?e', P, '?w', None))),
)


def solutions(query, facts):
    """Set-semantics BGP evaluation. `facts` are (s, p, o, statement) tuples."""
    out = [{}]
    for pattern in query:
        nxt = []
        for mu in out:
            for fact in facts:
                nu, ok = dict(mu), True
                for slot, value in zip(pattern, fact):
                    if slot is None:
                        continue
                    if isinstance(slot, str) and slot.startswith('?'):
                        if nu.setdefault(slot, value) != value:
                            ok = False
                            break
                    elif slot != value:
                        ok = False
                        break
                if ok:
                    nxt.append(nu)
        out = nxt
    return {tuple(sorted(mu.items())) for mu in out}


MERGED = [0]


def query_check(rows, graph, mapping):
    """Proposition (query view): asserted RDF-star triples are the q-image of the records,
    q is injective iff contents are distinct, and q maps BGP solutions onto RDF-star ones."""
    q, term = quoting(rows, mapping)
    image = {i: (term(s), mapping[p], term(o)) for i, (s, p, o) in rows.items()}
    assert asserted_triples(graph) == set(image.values())
    unique = len(set(rows.values())) == len(rows)
    assert (len(set(q.values())) == len(q)) == unique
    store = [row + (ref(i),) for i, row in rows.items()]
    star = [t + (('Q', t),) for t in image.values()]
    lift = lambda v: term(v) if v[0] == 'E' or v in mapping else v
    for query in QUERIES:
        here = solutions(query, store)
        mapped_query = tuple(tuple(lift(x) if isinstance(x, tuple) else x for x in pat)
                             for pat in query)
        there = solutions(mapped_query, star)
        moved = {tuple(sorted((k, lift(v)) for k, v in mu)) for mu in here}
        # q maps store answers into RDF-star answers, onto and one-to-one when contents differ.
        assert moved <= there
        if unique:
            assert moved == there and len(here) == len(there)
        else:
            MERGED[0] += moved != there


# --------------------------------------------------------------------------- RDF 1.2

def eid_iri(i):
    return ('I', 'eid:' + repr(i))


def encode_rdf12(rows, asserted):
    """Reifier encoding: (e rdf:reifies <<( s p o )>>) per record, eids named by IRIs,
    plus the plain triple for asserted records. Needs neither closure nor acyclicity."""
    terms = {t for row in rows.values() for t in row if t[0] != 'E'}
    mapping = term_map(terms)
    term = lambda t: eid_iri(t[1]) if t[0] == 'E' else mapping[t]
    triples = set()
    for i, (s, p, o) in rows.items():
        tt = (term(s), term(p), term(o))
        assert tt[0][0] in ('I', 'B') and tt[1][0] == 'I'
        triples.add((eid_iri(i), REIFIES, ('TT', tt)))
        if i in asserted:
            triples.add(tt)
    return triples, {v: k for k, v in mapping.items()}


def decode_rdf12(triples, inverse):
    back = lambda t: ref(literal_eval(t[1][4:])) if t[0] == 'I' and t[1].startswith('eid:') else inverse[t]
    rows, flags = {}, set()
    for s, p, o in triples:
        if p == REIFIES:
            i = literal_eval(s[1][4:])
            rows[i] = tuple(back(t) for t in o[1])
    for s, p, o in triples:
        if p == REIFIES:
            if o[1] in triples:
                flags.add(literal_eval(s[1][4:]))
    return rows, flags


def rdf12_check(rows, asserted):
    triples, inverse = encode_rdf12(rows, asserted)
    back, flags = decode_rdf12(triples, inverse)
    assert back == rows
    ambiguous = any(rows[i] == rows[j] for i in asserted for j in rows if j not in asserted)
    assert (flags == asserted) == (not ambiguous)
    return ambiguous


# --------------------------------------------------------------------------- LPG branch

def lpg_type(term):
    """Role-specific conversion g: nodes -> N, P -> relationship label set, Q -> key, literals -> V."""
    if term[0] in ('N', 'X'):
        return 'node'
    if term == P:
        return 'labels'
    if term == Q:
        return 'key'
    if term[0] == 'L':
        return 'value'
    return None


def lpg_image_valid(rows):
    """Gelling et al. Def. 5.3 on SG(rows): role typing plus edge/property statement kinds."""
    incoming = {i: 0 for i in rows}
    for row in rows.values():
        for t in (row[0], row[2]):
            if t[0] == 'E':
                incoming[t[1]] += 1
    kind = lambda t: 'stmt' if t[0] == 'E' else lpg_type(t)
    for i, (s, p, o) in rows.items():
        ks, kp, ko = kind(s), kind(p), kind(o)
        if ks not in ('node', 'stmt') or kp not in ('key', 'labels') or ko not in ('value', 'node', 'labels'):
            return False
        if kp == 'labels' and not (ks == 'node' and ko == 'node'):
            return False
        if kp == 'key' and not (ko == 'value' and not incoming[i]):
            return False
    return True


def lpg_direct(rows):
    """Characterization: edges between node terms; unreferenced properties on nodes or edges."""
    edge = lambda r: r[1] == P and lpg_type(r[0]) == 'node' and lpg_type(r[2]) == 'node'
    referenced = {t[1] for r in rows.values() for t in (r[0], r[2]) if t[0] == 'E'}
    for i, r in rows.items():
        if edge(r):
            continue
        if r[1] != Q or lpg_type(r[2]) != 'value' or i in referenced:
            return False
        if not (lpg_type(r[0]) == 'node' or (r[0][0] == 'E' and edge(rows[r[0][1]]))):
            return False
    return True


# --------------------------------------------------------------------------- checks

def check(rows, with_oracle=True):
    result = kernel(rows)
    peeled, order = peel(rows)
    assert peeled.keys() == result.keys() == fixpoint_iterates(rows), rows
    position = {i: k for k, i in enumerate(order)}
    assert all(position[j] < position[i] for i in order for j in targets(rows[i]))
    if with_oracle:
        assert set(result) == oracle_greatest(rows), rows
    assert kernel(result) == result
    graph, leaves, inverse = encode(result)
    assert image_valid(graph, leaves) and identity_preserving(result, graph)
    assert decode(graph, inverse) == result
    unique = len(set(result.values())) == len(result)
    assert image_valid(graph, leaves, rdf_star=True) == unique
    assert (result == rows) == oracle_admissible(rows, set(rows))
    query_check(result, graph, term_map({t for r in result.values() for t in r if t[0] != 'E'}))
    assert lpg_image_valid(result) == lpg_direct(result)
    rdf12_check(rows, {i for k, i in enumerate(sorted(rows, key=repr)) if k % 2 == 0})
    return result == rows


def leaf_substitution_check():
    """A dangling eid read as a leaf yields a Statement Graph that is not identity preserving."""
    rows = {0: (NODE, P, ref(7))}
    graph = {ref(0): (('I', 'encoded:N'), P, ('I', 'dangling:7')),
             ref(('membership', 0)): (ref(0), IN, DEFAULT)}
    leaves = {('I', 'encoded:N'), P, ('I', 'dangling:7'), IN, DEFAULT}
    assert image_valid(graph, leaves)
    assert not identity_preserving(rows, graph)
    return rows


def canonical(structure):
    """Smallest relabeling of a reference structure on records 0..n-1 (-1: no eid, n: missing)."""
    n = len(structure)
    best = None
    for perm in permutations(range(n)):
        relabel = lambda x: x if x < 0 or x == n else perm[x]
        image = [None] * n
        for i, (s, o) in enumerate(structure):
            image[perm[i]] = (relabel(s), relabel(o))
        image = tuple(image)
        if best is None or image < best:
            best = image
    return best


def four_record_structures():
    """All reference structures on four records, up to relabeling, with one missing eid."""
    n = 4
    endpoints = [-1] + list(range(n + 1))
    seen = {structure for structure in product(product(endpoints, endpoints), repeat=n)
            if canonical(structure) == structure}
    tested = complete = 0
    for structure in seen:
        rows = {i: (NODE if s < 0 else ref(s), P, LITERAL if o < 0 else ref(o))
                for i, (s, o) in enumerate(structure)}
        complete += check(rows)
        tested += 1
    return tested, complete


def random_stores(count, rng):
    complete = 0
    for _ in range(count):
        n = rng.randint(4, 7)
        ids = list(range(n + 2))
        rows = {}
        for i in range(n):
            s = rng.choice([NODE, NODE2] + [ref(rng.choice(ids)) for _ in range(2)])
            o = rng.choice([NODE, LITERAL] + [ref(rng.choice(ids)) for _ in range(2)])
            rows[i] = (s, rng.choice((P, Q)), o)
        complete += check(rows)
    return complete


# --------------------------------------------------------------------------- protocol model

class Store:
    """Restricted protocol plus engine-style supersession (replay with fresh eids + lineage)."""

    def __init__(self, strict=False):
        self.strict = strict
        self.rejected = 0
        self.rows, self.t, self.next, self.batch = {}, 0, 0, {}
        self.exportable = {}
        self.supersessions = 0

    def live(self, t=None):
        t = self.t if t is None else t
        return {i for i, r in self.rows.items() if r[3] <= t and (r[4] is None or t < r[4])}

    def content(self):
        return {i: r[:3] for i, r in self.rows.items()}

    def insert(self, s, p, o, v1, v2, batch):
        i, self.next = self.next, self.next + 1
        self.rows[i] = (s, p, o, self.t, None, v1, v2)
        self.batch[i] = batch
        return i

    def retract(self, i):
        r = self.rows[i]
        self.rows[i] = r[:4] + (self.t,) + r[5:]

    def cascade(self, root, live):
        out, stack = set(), [root]
        while stack:
            i = stack.pop()
            if i in out:
                continue
            out.add(i)
            stack.extend(j for j in live if i in targets(self.rows[j][:3]))
        return out

    def mark(self, ids):
        """Incremental exportable bit: a peel over the new rows with fixed external bits."""
        pending = set(ids)
        changed = True
        while changed:
            changed = False
            for i in list(pending):
                ts = targets(self.rows[i][:3])
                if all(self.exportable.get(j) for j in ts):
                    self.exportable[i] = True
                    pending.discard(i)
                    changed = True
        for i in pending:
            self.exportable[i] = False

    def step(self, rng):
        self.t += 1
        live = sorted(self.live(self.t - 1))
        op = rng.random()
        if not live or op < 0.45:
            pick = lambda base: rng.choice(base + [ref(i) for i in live]) if live else rng.choice(base)
            v1 = rng.choice([None, 0, 5])
            i = self.insert(pick([NODE, NODE2]), rng.choice((P, Q)), pick([NODE, LITERAL]),
                            v1, rng.choice([None, 10]), ('insert', self.t))
            self.mark([i])
        elif op < 0.7:
            for j in self.cascade(rng.choice(live), set(live)):
                self.retract(j)
        else:
            root = rng.choice(live)
            members = self.cascade(root, set(live))
            sigma = {j: self.next + k for k, j in enumerate(sorted(members))}
            self.next += len(members)
            candidates = [NODE2, LITERAL] + [ref(j) for j in live if j != root]
            patch = rng.choice(candidates)
            if patch[0] == 'E' and patch[1] in members:
                patch = ref(sigma[patch[1]])
            remap = lambda x: ref(sigma[x[1]]) if x[0] == 'E' and x[1] in sigma else x
            if self.strict:
                # Acyclic-replay rule: peel the replay batch alone before writing anything.
                batch = {sigma[j]: (remap(self.rows[j][0]), self.rows[j][1],
                                    patch if j == root else remap(self.rows[j][2]))
                         for j in members}
                inside = {k: tuple(t if t[0] != 'E' or t[1] in batch else NODE for t in r)
                          for k, r in batch.items()}
                cyclic = len(peel(inside)[0]) < len(inside)
                # The rule rejects exactly the patches that name a member of the cascade set.
                assert cyclic == (patch[0] == 'E' and patch[1] in batch)
                if cyclic:
                    self.next -= len(members)
                    self.rejected += 1
                    return
            for j in members:
                self.retract(j)
            for j in sorted(members):
                s, p, o, _, _, v1, v2 = self.rows[j]
                o = patch if j == root else remap(o)
                self.rows[sigma[j]] = (remap(s), p, o, self.t, None, v1, v2)
                self.batch[sigma[j]] = ('replay', self.t)
            self.mark(list(sigma.values()))
            lineage = self.insert(ref(sigma[root]), SUPERSEDES, ref(root), None, None,
                                  ('lineage', self.t))
            self.mark([lineage])
            self.supersessions += 1


def cycle_vertices(rows):
    adjacency = {i: targets(r) & rows.keys() for i, r in rows.items()}
    out = set()
    for start in rows:
        stack, seen = list(adjacency[start]), set()
        while stack:
            i = stack.pop()
            if i == start:
                out.add(start)
                break
            if i not in seen:
                seen.add(i)
                stack.extend(adjacency[i])
    return out


def strict_runs(count, steps, rng):
    """Under the acyclic-replay rule D stays acyclic, K(D) = D and every bit is set."""
    rejected = 0
    for _ in range(count):
        store = Store(strict=True)
        for _ in range(steps):
            store.step(rng)
            content = store.content()
            assert closed(content) and not cycle_vertices(content)
            assert kernel(content) == content
            assert all(store.exportable.values())
        rejected += store.rejected
    return rejected


def protocol_runs(count, steps, rng):
    snapshots = histories_with_cycles = 0
    for _ in range(count):
        store = Store()
        for _ in range(steps):
            store.step(rng)
            content = store.content()
            # Full history is closed (Proposition: closed history).
            assert closed(content)
            # Every cycle lies inside one replay batch.
            cyc = cycle_vertices(content)
            for i in cyc:
                for j in cyc:
                    if j in reference_closure({i}, content) and i in reference_closure({j}, content):
                        assert store.batch[i] == store.batch[j] and store.batch[i][0] == 'replay'
            histories_with_cycles += bool(cyc)
            # K(D) removes exactly cycle vertices and their dependents.
            k = kernel(content)
            removed = content.keys() - k.keys()
            assert removed == {i for i in content
                               if reference_closure({i}, content) & cyc}
            # The incremental bit selects K(D): export is a filter scan.
            assert {i for i, bit in store.exportable.items() if bit} == k.keys()
            # Without supersession every committed snapshot is closed and D is acyclic.
            for t in range(store.t + 1):
                snap = {i: content[i] for i in store.live(t)}
                if store.supersessions == 0:
                    assert closed(snap) and not cyc
                # On a closed snapshot the kernel is the snapshot filtered by the history bit.
                if closed(snap):
                    assert kernel(snap).keys() == snap.keys() & k.keys()
                # The dual construction exists in D and is the least closed superset.
                up = reference_closure(snap.keys(), content)
                assert up is not None and closed({i: content[i] for i in up})
                assert all(store.rows[i][3] <= t for i in up)
                assert not (up - snap.keys()) & store.live(t)
                ks, kup = kernel(snap).keys(), kernel({i: content[i] for i in up}).keys()
                assert ks <= kup <= up
                for i in (kup & snap.keys()) - ks:
                    assert reference_closure({i}, content) - snap.keys()
                snapshots += 1
            # Lossless temporal roundtrip of the whole exportable history.
            records = {i: store.rows[i] for i in k}
            asserted = store.live() & k.keys()
            graph, leaves, inverse = encode_temporal(records, asserted)
            assert image_valid(graph, leaves)
            back, flags = decode_temporal(graph, inverse)
            assert back == records and flags == asserted
    return snapshots, histories_with_cycles


def dual_minimality(count, rng):
    """C(S) equals the intersection of all closed supersets of S in a closed store."""
    checked = 0
    for _ in range(count):
        n = rng.randint(1, 6)
        rows = {}
        for i in range(n):
            s = rng.choice([NODE] + [ref(j) for j in range(i)]) if i else NODE
            o = rng.choice([LITERAL] + [ref(j) for j in range(i)]) if i else LITERAL
            rows[i] = (s, P, o)
        chosen = {i for i in rows if rng.random() < 0.4}
        up = reference_closure(chosen, rows)
        supersets = [set(i for b, i in enumerate(rows) if mask >> b & 1)
                     for mask in range(1 << n)]
        supersets = [x for x in supersets if chosen <= x and closed({i: rows[i] for i in x})]
        assert up == set.intersection(*supersets)
        checked += 1
    return checked


# --------------------------------------------------------------------------- focused cases

def focused_checks():
    cases = {
        'historical_lineage': {1: (NODE, P, NODE), 2: (ref(1), SUPERSEDES, ref(0))},
        'correction_cycle': {0: (NODE, P, ref(1)), 1: (ref(0), Q, LITERAL)},
        'cycle_with_unrelated_fact': {0: (NODE, P, ref(1)), 1: (ref(0), Q, LITERAL), 2: (NODE, P, NODE)},
        'annotated_membership': {0: (NODE, P, NODE), 1: (ref(0), ('I', 'inGraph'), NODE), 2: (ref(1), Q, LITERAL)},
        'statement_graph_name': {0: (NODE, P, NODE), 1: (NODE, Q, NODE), 2: (ref(0), ('I', 'inGraph'), ref(1))},
        'mixed_term_roles': {0: (P, P, NODE), 1: (NODE, P, LITERAL)},
        'transaction_and_blank_terms': {0: (('X', 42), P, ('B', 'b')), 1: (ref(0), Q, ('X', 42))},
        'reserved_source_iri': {0: (IN, IN, DEFAULT)},
        'duplicate_occurrences': {0: (NODE, P, NODE), 1: (NODE, P, NODE)},
        'annotation_of_removed_lineage': {0: (NODE, P, NODE), 1: (ref(0), P, ref(9)), 2: (ref(1), Q, LITERAL)},
    }
    for rows in cases.values():
        check(rows)
    assert set(kernel(cases['historical_lineage'])) == {1}
    assert not kernel(cases['correction_cycle'])
    assert set(kernel(cases['cycle_with_unrelated_fact'])) == {2}
    assert set(kernel(cases['annotation_of_removed_lineage'])) == {0}
    # Example 1 as full history: the retracted target is present, so nothing is lost.
    history = {**cases['historical_lineage'], 0: (NODE, P, NODE)}
    assert kernel(history) == history
    leaf_substitution_check()
    intervals = {0: (1, 2), 1: (2, 2)}
    snap = lambda t: {i for i, (a, d) in intervals.items() if a <= t < d}
    assert snap(1) == {0} and snap(2) == set()
    assert (snap(1) | {1}) - {0, 1} == snap(2)
    g, leaves, _ = encode({0: (NODE, P, NODE)})
    mutations = [({ref(0): g[ref(0)]}, leaves)]
    h = dict(g); h[ref(0)] = (ref(0), h[ref(0)][1], h[ref(0)][2]); mutations.append((h, leaves))
    h = dict(g); h[ref(0)] = (g[ref(0)][0], ref(0), g[ref(0)][2]); mutations.append((h, leaves))
    mutations.append((g, leaves | {('I', 'unused')}))
    h = dict(g); h[ref(0)] = (ref(999), g[ref(0)][1], g[ref(0)][2]); mutations.append((h, leaves))
    assert all(not image_valid(h, terms) for h, terms in mutations)
    return len(cases), len(mutations)


def engine_trace():
    """Recompute kernels on rows dumped by the real engine and compare with the expectations."""
    path = ROOT / 'engine-trace.json'
    if not path.exists():
        return None
    data = json.loads(path.read_text())
    for name in ('lineage', 'correction_cycle'):
        case = data[name]
        term = lambda x: ref(x[1]) if x[0] == 'E' else ('T', x[1])
        content = {r['e']: (term(r['s']), term(r['p']), term(r['o'])) for r in case['history']}
        now = {i: content[i] for i in case['now_eids']}
        assert closed(content) and not closed(now)
        assert sorted(kernel(content)) == case['kernel_history']
        assert sorted(kernel(now)) == case['kernel_now']
        records = {r['e']: content[r['e']] + (r['a'], r['d'], r['v1'], r['v2']) for r in case['history']}
        k = {i: records[i] for i in kernel(content)}
        graph, leaves, inverse = encode_temporal(k, set(case['now_eids']) & k.keys())
        assert image_valid(graph, leaves)
        assert decode_temporal(graph, inverse) == (k, set(case['now_eids']) & k.keys())
        # The whole history, cycle included, roundtrips through RDF 1.2 reifiers; the asserted
        # set is ambiguous exactly in Example 1, where e1' repeats the content of retracted e1.
        assert rdf12_check(content, set(case['now_eids'])) == (name == 'lineage')
    assert data['lineage']['kernel_history'] == [1, 2, 3, 4, 5]
    assert data['lineage']['kernel_now'] == [3, 4]
    assert data['correction_cycle']['kernel_history'] == [1, 2]
    assert data['correction_cycle']['kernel_now'] == []
    return {'engine': data['engine'],
            'sha256': hashlib.sha256(path.read_bytes()).hexdigest()}


def main():
    counts = []
    for n in range(4):
        endpoints = [NODE] + [ref(i) for i in range(n + 1)]
        choices = list(product(endpoints, (P, Q), endpoints + [LITERAL]))
        tested = complete = 0
        for values in product(choices, repeat=n):
            complete += check(dict(enumerate(values)))
            tested += 1
        counts.append({'records': n, 'candidates': tested, 'complete_exports': complete})
        print(counts[-1], flush=True)
    structures, structures_complete = four_record_structures()
    print({'four_record_structures': structures}, flush=True)
    rng = random.Random(20261011)
    random_count = 20000
    random_complete = random_stores(random_count, rng)
    runs, steps = 1500, 8
    snapshots, cyclic = protocol_runs(runs, steps, rng)
    dual = dual_minimality(3000, rng)
    strict_count = 1500
    strict_rejected = strict_runs(strict_count, steps, rng)
    focused, mutations = focused_checks()
    trace = engine_trace()
    total = sum(item['candidates'] for item in counts)
    summary = {'by_size': counts, 'candidate_stores': total,
               'four_record_structures': {'up_to_relabeling': structures,
                                          'complete_exports': structures_complete},
               'random_stores': {'count': random_count, 'records': '4-7',
                                 'complete_exports': random_complete},
               'protocol_histories': {'runs': runs, 'steps': steps,
                                      'snapshots_checked': snapshots,
                                      'states_with_cycles': cyclic},
               'dual_closure_checks': dual,
               'strict_policy_histories': {'runs': strict_count, 'steps': steps,
                                           'rejected_supersessions': strict_rejected},
               'duplicate_stores_with_merged_answers': MERGED[0],
               'per_store_checks': ['kernel', 'encoding', 'query view', 'RDF 1.2 reifiers',
                                    'LPG branch'],
               'focused_stores': focused, 'rejected_image_mutations': mutations,
               'same_transaction_boundary': 'passed', 'engine_trace': trace,
               'script_sha256': hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
               'scope': 'Finite models; engine evidence limited to the two traced examples.'}
    (ROOT / 'validation-results.json').write_text(json.dumps(summary, indent=2) + '\n')
    fmt = lambda x: f'{x:,}'.replace(',', '\\,')
    engine = ('the two engine traces were recomputed from rows dumped by ' + trace['engine'] + ', '
              if trace else 'no engine trace was present, ')
    (ROOT / 'validation-counts.tex').write_text(
        '\\par\\noindent The run checked \\textbf{' + fmt(total) + '} labeled candidate stores of '
        'up to three records, \\textbf{' + fmt(structures) + '} four-record reference structures up '
        'to relabeling, \\textbf{' + fmt(random_count) + '} random stores of four to seven records, '
        'and \\textbf{' + fmt(runs) + '} random protocol histories of ' + str(steps) + ' transactions ('
        + fmt(snapshots) + ' snapshot checks, ' + fmt(cyclic) + ' states containing replay cycles), '
        'and ' + fmt(strict_count) + ' histories under the acyclic-replay rule (' + fmt(strict_rejected)
        + ' supersessions rejected); '
        + fmt(dual) + ' reference-closure minimality checks, ' + str(focused) + ' focused store cases, '
        + str(mutations) + ' rejected image mutations and the same-transaction boundary also passed; '
        + engine + 'with no disagreements.\\par\n')
    print(json.dumps(summary, indent=2))


if __name__ == '__main__':
    main()
