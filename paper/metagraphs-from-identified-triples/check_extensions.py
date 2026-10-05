#!/usr/bin/env python3
"""Revision-2 checks: necessity witnesses, seeded randomized tests, firing closure,
strata/cascade, snapshot policies and version chains. Standard library only.

Imported by check_encodings.py; every function asserts and raises on failure.
"""
from collections import Counter, defaultdict, deque
from itertools import product
import random

import check_encodings as ce
from check_encodings import (model, encode, decode, validate, ranks, subsets, recognize,
                             oracle, roundtrip, is_stmt)

SEED = 20261005

# ---------------------------------------------------------------- necessity witnesses

def _ca(**over):
    base = {'v0': 'Vertex', 'm0': 'MetaVertex', 'e0': 'Edge', 'q0': 'MetaEdge'}
    return base | over

def _attr_base():
    return [('e0', 'src', 'v0'), ('e0', 'tgt', 'm0'), ('q0', 'src', 'v0'), ('q0', 'tgt', 'm0')], \
           [('e0', True), ('q0', False)]

def witnesses():
    """(constraint, family, statement, model) with a model violating only that constraint."""
    v2 = {'v0': 'Vertex', 'v1': 'Vertex', 'e0': 'Edge'}
    ok = [('e0', 'in', 'v0'), ('e0', 'out', 'v1')]
    two = v2 | {'e1': 'Edge'}
    ok2 = ok + [('e1', 'in', 'v0'), ('e1', 'out', 'v1')]
    roles, flags = _attr_base()
    ca = _ca()
    W = [
        ('D1', 'D', 'role subject is an Edge anchor', model(v2, ok + [('v0', 'in', 'v1')])),
        ('D2', 'D', 'role object is a Vertex anchor', model(two, ok2[:2] + [('e1', 'in', 'v0'), ('e1', 'out', 'v1'), ('e0', 'in', 'e1')])),
        ('D3', 'D', 'each edge has an in role', model(v2, [('e0', 'out', 'v1')])),
        ('D4', 'D', 'each edge has an out role', model(v2, [('e0', 'in', 'v0')])),
        ('D5', 'D', 'no extra annotation, direction or attribute rows', model(v2, ok, attrs=[('v0', 'attr:k', 'value')])),
        ('H2', 'HO', 'no edge is an end of itself', model(v2, ok + [('e0', 'in', 'e0')])),
        ('B1', 'BB', 'no two edges have equal in/out sets', model(two, ok2)),
        ('U1', 'U', 'only member roles', model({'v0': 'Vertex', 'e0': 'Edge'}, [('e0', 'member', 'v0'), ('e0', 'in', 'v0')])),
        ('U2', 'U', 'member sets are nonempty', model({'v0': 'Vertex', 'e0': 'Edge'})),
        ('U3', 'U', 'member relation is acyclic', model({'e0': 'Edge', 'e1': 'Edge'}, [('e0', 'member', 'e1'), ('e1', 'member', 'e0')])),
        ('U4', 'U', 'member sets are distinct (extensional)', model({'v0': 'Vertex', 'e0': 'Edge', 'e1': 'Edge'}, [('e0', 'member', 'v0'), ('e1', 'member', 'v0')])),
        ('A1', 'A', 'exactly one src per (meta)edge', model(ca, roles[1:], flags)),
        ('A2', 'A', 'exactly one tgt per (meta)edge', model(ca, roles + [('e0', 'tgt', 'v0')], flags)),
        ('A3', 'A', 'exactly one direction flag', model(ca, roles, flags[:1])),
        ('A4', 'A', 'src/tgt objects are Vertex or MetaVertex', model(ca, [('e0', 'src', 'q0')] + roles[1:], flags)),
        ('A5', 'A', 'membership objects are containers', model(ca, roles + [('e0', 'inGraph', 'v0')], flags)),
        ('A6', 'A', 'membership subject differs from object', model(ca, roles + [('m0', 'inGraph', 'm0')], flags)),
        ('A7', 'A', 'attribute keys are not reserved', model(ca, roles, flags, [('v0', 'in', 'value')])),
    ]
    return W

FAMILY_CONSTRAINTS = {
    'D': ['D1', 'D2', 'D3', 'D4', 'D5'],
    'HO': ['D1', 'D3', 'D4', 'D5', 'H2'],
    'BB': ['D1', 'D2', 'D3', 'D4', 'D5', 'B1'],
    'U': ['U1', 'U2', 'U3', 'U4'],
    'A': ['A1', 'A2', 'A3', 'A4', 'A5', 'A6', 'A7'],
}

def necessity_checks():
    n = 0
    for name, family, _, m in witnesses():
        try:
            validate(m, family)
        except ValueError:
            pass
        else:
            raise AssertionError(('witness accepted', name))
        validate(m, family, drop=(name,))
        for other in FAMILY_CONSTRAINTS[family]:
            if other != name:
                try:
                    validate(m, family, drop=(other,))
                except ValueError:
                    continue
                raise AssertionError(('witness violates more than one constraint', name, other))
        n += 1
    return n

def ablation_table_tex():
    rows = []
    for name, family, text, m in witnesses():
        label = {'D': r'$\Img{D}$', 'HO': r'$\Img{HO}$', 'BB': r'$\Img{BB}$',
                 'U': r'$\Img{U}$', 'A': r'$\Img{A}$'}[family]
        if name.startswith('D'):
            mark = '(D%s)' % name[1:]
        else:
            mark = '(%s)' % name
        rows.append((mark, label, text, witness_text(name)))
    lines = [r'\begin{tabular}{llL{4.6cm}L{5.2cm}}', r'\toprule',
             r'Constraint & Image & Requirement & Witness violating only it \\', r'\midrule']
    lines += [' & '.join(r) + r' \\' for r in rows]
    lines += [r'\bottomrule', r'\end{tabular}']
    return '\n'.join(lines) + '\n'

def witness_text(name):
    return {
        'D1': r'a role row whose subject is a Vertex anchor',
        'D2': r'an in-role whose object is an Edge anchor',
        'D3': r'an edge with only an out role',
        'D4': r'an edge with only an in role',
        'D5': r'an attribute row on a Vertex',
        'H2': r'$e$ with an in-role to $e$ itself',
        'B1': r'two edges with equal in and out sets',
        'U1': r'an in-role in a member structure',
        'U2': r'an edge with no member',
        'U3': r'$e_0$ a member of $e_1$ and $e_1$ of $e_0$',
        'U4': r'two edges with the same single member',
        'A1': r'a metaedge with no src row',
        'A2': r'an edge with two tgt rows',
        'A3': r'a metaedge with no direction row',
        'A4': r'a src role to a MetaEdge',
        'A5': r'a membership into a Vertex',
        'A6': r'a MetaVertex member of itself',
        'A7': r'attribute key \texttt{mg:in}',
    }[name]

# ---------------------------------------------------------------- random models

def rand_subset(rng, pool, nonempty=True):
    pool = sorted(pool)
    while True:
        s = frozenset(x for x in pool if rng.random() < 0.4)
        if s or not nonempty:
            return s
        if pool and rng.random() < 0.5:
            return frozenset([rng.choice(pool)])

def rand_directed(rng, family, maxv=8, maxe=8):
    nv, ne = rng.randint(1, maxv), rng.randint(0, maxe)
    c = {f'v{i}': 'Vertex' for i in range(nv)} | {f'e{i}': 'Edge' for i in range(ne)}
    roles, seen = [], set()
    for i in range(ne):
        e = f'e{i}'
        pool = {a for a in c if a != e} if family == 'HO' else {a for a in c if c[a] == 'Vertex'}
        V, W = rand_subset(rng, pool), rand_subset(rng, pool)
        if family == 'BB':
            if (V, W) in seen:
                continue
            seen.add((V, W))
        roles += [(e, 'in', x) for x in V] + [(e, 'out', y) for y in W]
    # edges skipped by BB dedup would have no roles: drop them
    have = {a for a, p, b in roles}
    c = {a: k for a, k in c.items() if k == 'Vertex' or a in have}
    return model(c, roles)

def rand_uber(rng, maxv=6, maxe=6):
    nv, ne = rng.randint(1, maxv), rng.randint(0, maxe)
    c = {f'v{i}': 'Vertex' for i in range(nv)}
    roles, seen = [], set()
    for i in range(ne):
        pool = list(c)
        M = rand_subset(rng, pool)
        if M in seen:
            continue
        seen.add(M)
        e = f'e{i}'
        c[e] = 'Edge'
        roles += [(e, 'member', a) for a in M]
    return model(c, roles)

def rand_attributed(rng):
    c = {}
    for k, pre in (('Vertex', 'v'), ('MetaVertex', 'm'), ('Edge', 'e'), ('MetaEdge', 'q')):
        for i in range(rng.randint(1, 3)):
            c[f'{pre}{i}'] = k
    vs = [a for a, k in c.items() if k in {'Vertex', 'MetaVertex'}]
    es = [a for a, k in c.items() if k in {'Edge', 'MetaEdge'}]
    cs = [a for a, k in c.items() if k in {'MetaVertex', 'MetaEdge'}]
    roles = [(e, p, rng.choice(vs)) for e in es for p in ('src', 'tgt')]
    for ctn in cs:
        roles += [(b, 'inGraph', ctn) for b in c if b != ctn and rng.random() < 0.3]
    flags = [(e, rng.random() < 0.5) for e in es]
    attrs = [(a, f'attr:k{j}', f'val{j}') for a in c for j in range(2) if rng.random() < 0.3]
    return model(c, roles, flags, attrs)

def canon(rows):
    """Canonical multiset of contents: references to anchor rows become node references."""
    handle = {sid: s for sid, (s, p, o) in rows.items() if p == 'type'}
    f = lambda x: ('H', handle[x]) if is_stmt(x) and x in handle else x
    return Counter((f(s), p, f(o)) for s, p, o in rows.values())

def mutate(rng, rows):
    rows = dict(rows)
    ids = sorted(rows)
    kind = rng.choice(['delete', 'dup', 'retarget', 'repredicate', 'extra', 'extra_anchor'])
    if not ids:
        kind = 'extra'
    if kind == 'delete':
        del rows[rng.choice(ids)]
    elif kind == 'dup':
        rows[('s', 'dup')] = rows[rng.choice(ids)]
    elif kind == 'retarget':
        sid = rng.choice(ids)
        s, p, o = rows[sid]
        rows[sid] = (s, p, rng.choice(ids)) if p in {'in', 'out', 'member', 'src', 'tgt', 'inGraph'} else (s, p, o)
    elif kind == 'repredicate':
        sid = rng.choice(ids)
        s, p, o = rows[sid]
        rows[sid] = (s, rng.choice(['in', 'out', 'member', 'src', 'tgt', 'inGraph', 'directed', 'relatedTo']), o)
    elif kind == 'extra':
        a = rng.choice(ids) if ids else ('s', 0)
        rows[('s', 'extra')] = (a, rng.choice(['in', 'out', 'relatedTo', 'member']), rng.choice(ids) if ids else ('s', 0))
    else:
        rows[('s', 'extra')] = (('n', 'fresh'), 'type', ('class', rng.choice(ce.KINDS)))
    return rows

def randomized_checks(rng, counts):
    gens = [('D', lambda: rand_directed(rng, 'D')), ('HO', lambda: rand_directed(rng, 'HO')),
            ('BB', lambda: rand_directed(rng, 'BB')), ('U', lambda: rand_uber(rng)),
            ('A', lambda: rand_attributed(rng))]
    for family, gen in gens:
        for _ in range(1500):
            m = gen()
            validate(m, family)
            roundtrip(m, family)
            counts['random_roundtrips'] += 1
            rows = encode(m)
            for _ in range(2):
                mutated = mutate(rng, rows)
                try:
                    decoded = decode(mutated, family)
                except ValueError:
                    counts['mutated_rejected'] += 1
                else:
                    assert canon(encode(decoded)) == canon(mutated), (family, mutated)
                    counts['mutated_accepted'] += 1

# ---------------------------------------------------------------- strata and cascade

def rand_f4_store(rng):
    m = rand_attributed(rng) if rng.random() < 0.5 else rand_directed(rng, 'HO', 5, 5)
    rows = encode(m)
    for i in range(rng.randint(0, 12)):
        ids = sorted(rows, key=str)
        a = rng.choice(ids)
        b = rng.choice(ids) if rng.random() < 0.3 else ('literal', i)
        o = b if not is_stmt(b) or b != a else ('literal', i)
        rows[('s', f'ann{i}')] = (a, f'ann{rng.randint(0, 2)}', o)
    return rows

def refs_of(rows):
    return {sid: [x for x in (s, o) if is_stmt(x)] for sid, (s, p, o) in rows.items()}

def longest_chain(rows):
    refs = refs_of(rows)
    memo = {}
    def go(s):
        if s not in memo:
            memo[s] = 1 + max((go(r) for r in refs[s]), default=0)
        return memo[s]
    return max((go(s) for s in rows), default=0)

def strata_and_cascade_checks(rng, counts):
    for _ in range(1500):
        rows = rand_f4_store(rng)
        rk = ranks(rows)
        top = max(rk.values(), default=0)
        refs = refs_of(rows)
        # partition by rank: every endpoint in a strictly earlier part
        assert all(rk[r] < rk[s] for s in rows for r in refs[s])
        # minimality: a chain with top+1 rows exists (so fewer parts are impossible)
        assert longest_chain(rows) == top + 1 if rows else top == 0
        counts['strata_checks'] += 1
        e = rng.choice(sorted(rows, key=str))
        # reverse closure computed by rounds
        deleted, frontier, rounds = {e}, {e}, 0
        while True:
            new = {s for s in rows if s not in deleted and any(r in frontier for r in refs[s])}
            if not new:
                break
            deleted |= new
            frontier = new
            rounds += 1
        assert rounds <= top - rk[e]
        assert all(rk[d] > rk[e] for d in deleted if d != e)
        # fixpoint characterization of dependent-row cascade
        fixed = {e}
        changed = True
        while changed:
            changed = False
            for s in rows:
                if s not in fixed and any(r in fixed for r in refs[s]):
                    fixed.add(s); changed = True
        assert fixed == deleted
        counts['cascade_checks'] += 1

# ---------------------------------------------------------------- firing closure

def vw(m):
    es = sorted(a for a, k in m.classes if k == 'Edge')
    V = {e: frozenset(b for a, p, b in m.roles if a == e and p == 'in') for e in es}
    W = {e: frozenset(b for a, p, b in m.roles if a == e and p == 'out') for e in es}
    return V, W

def closure_fast(V, W, F, B):
    """Counter-based forward chaining, linear in the number of arcs."""
    feeds = defaultdict(list)
    need = {}
    for e in F:
        need[e] = len(V[e])
        for x in V[e]:
            feeds[x].append(e)
    reached, queue, fired = set(B), deque(B), set()
    while queue:
        x = queue.popleft()
        for e in feeds[x]:
            need[e] -= 1
            if need[e] == 0:
                fired.add(e)
                for y in W[e]:
                    if y not in reached:
                        reached.add(y); queue.append(y)
    return reached, fired

def closure_naive(V, W, F, B):
    reached = set(B)
    while True:
        add = set()
        for e in F:
            if V[e] <= reached:
                add |= W[e]
        if add <= reached:
            return reached
        reached |= add

def walk_reach(V, W, F, B):
    """y such that some edge sequence in F starts at V_e meeting B and ends with y in W_e."""
    seen_e, frontier = set(), {e for e in F if V[e] & set(B)}
    out = set()
    while frontier:
        seen_e |= frontier
        nxt = set()
        for e in frontier:
            out |= W[e]
            nxt |= {e2 for e2 in F if V[e2] & W[e] and e2 not in seen_e}
        frontier = nxt
    return out

def dep_acyclic(V, W, F):
    adj = {e: [e2 for e2 in F if W[e] & V[e2]] for e in F}
    return ce.acyclic(set(F), adj)

def exec_metapath(m, V, W, B, C, F):
    reached, fired = closure_fast(V, W, F, B)
    return recognize(m, B, C, F) and fired == set(F)

def firing_case(m, V, W, B, C, F, counts, check_oracle=False):
    reached, fired = closure_fast(V, W, F, B)
    assert reached == closure_naive(V, W, F, B)
    assert fired == {e for e in F if V[e] <= reached}
    wr = walk_reach(V, W, F, B)
    assert reached <= set(B) | wr                                  # Theorem exec (a)
    singleton = all(len(V[e]) == 1 for e in F)
    if singleton:
        assert reached == set(B) | wr                              # Theorem exec (b)
    metapath = recognize(m, B, C, F)
    if check_oracle and len(F) <= 3:
        assert metapath == oracle(m, B, C, F)
    if metapath:
        counts['firing_metapaths'] += 1
        if singleton or dep_acyclic(V, W, F):
            assert fired == set(F)                                  # Theorem coincide
            counts['firing_coincide_antecedent'] += 1
        elif fired != set(F):
            counts['firing_strict_gap'] += 1
            assert not singleton and not dep_acyclic(V, W, F)
    counts['firing_cases'] += 1

def firing_checks(rng, counts):
    # the paper's counterexample
    ex = model({'a': 'Vertex', 'b': 'Vertex', 'c': 'Vertex', 'd': 'Vertex', 'e1': 'Edge', 'e2': 'Edge'},
               [('e1', 'in', 'b'), ('e1', 'in', 'a'), ('e1', 'out', 'd'),
                ('e2', 'in', 'd'), ('e2', 'out', 'a'), ('e2', 'out', 'c')])
    V, W = vw(ex)
    F = {'e1', 'e2'}
    assert recognize(ex, {'b'}, {'c'}, F) and closure_fast(V, W, F, {'b'})[0] == {'b'}
    assert not dep_acyclic(V, W, F)
    counts['gap_example'] += 1
    # exhaustive: 3 vertices, at most 2 edges, nonempty ends
    xs = ['v0', 'v1', 'v2']
    ends = subsets(set(xs), True)
    for n in range(3):
        c = {x: 'Vertex' for x in xs} | {f'e{i}': 'Edge' for i in range(n)}
        for pairs in product(product(ends, ends), repeat=n):
            m = model(c, [(f'e{i}', p, y) for i, (a, b) in enumerate(pairs)
                          for p, ys in (('in', a), ('out', b)) for y in ys])
            V, W = vw(m)
            es = sorted(V)
            for B, C, F in product(subsets(set(xs)), subsets(set(xs)), subsets(set(es))):
                firing_case(m, V, W, B, C, F, counts)
                counts['firing_exhaustive'] += 1
    # randomized, larger
    for _ in range(4000):
        nv, ne = rng.randint(2, 6), rng.randint(1, 4)
        xs = [f'v{i}' for i in range(nv)]
        c = {x: 'Vertex' for x in xs} | {f'e{i}': 'Edge' for i in range(ne)}
        roles = []
        for i in range(ne):
            Vs = rand_subset(rng, xs)
            if rng.random() < 0.4:
                Vs = frozenset([rng.choice(xs)])
            roles += [(f'e{i}', 'in', x) for x in Vs] + [(f'e{i}', 'out', y) for y in rand_subset(rng, xs)]
        m = model(c, roles)
        V, W = vw(m)
        B, C = rand_subset(rng, xs, False), rand_subset(rng, xs, False)
        F = frozenset(e for e in V if rng.random() < 0.8)
        firing_case(m, V, W, B, C, F, counts, check_oracle=True)
        counts['firing_random'] += 1
    assert counts['firing_strict_gap'] > 0 and counts['firing_coincide_antecedent'] > 0

# ---------------------------------------------------------------- temporal policies

COMPONENTS = ['v0', 'm0', 'e0', 'membership', 'attribute', 'assessment', 'audit']
DEPS = {'v0': set(), 'm0': set(), 'e0': {'v0', 'm0'}, 'membership': {'v0', 'm0'},
        'attribute': {'v0'}, 'assessment': {'membership'}, 'audit': {'assessment'}}
HORIZON = 4

def interval_within(rng, lo, hi):
    if hi - lo < 1:
        return None
    a = rng.randint(lo, hi - 1)
    return (a, rng.randint(a + 1, hi))

def rand_history(rng, containment):
    """Clocks for the seven components; transaction containment always, valid containment if asked."""
    L, J = {}, {}
    for c in COMPONENTS:                                       # topological order
        ps = DEPS[c]
        llo = max((L[p][0] for p in ps), default=0)
        lhi = min((L[p][1] for p in ps), default=HORIZON)
        li = interval_within(rng, llo, lhi)
        if li is None:
            return None
        L[c] = li
        if containment:
            jlo = max((J[p][0] for p in ps), default=0)
            jhi = min((J[p][1] for p in ps), default=HORIZON)
            ji = interval_within(rng, jlo, jhi)
            if ji is None:
                return None
            J[c] = ji
        else:
            J[c] = interval_within(rng, 0, HORIZON)
    return L, J

def live(iv, x):
    return iv[0] <= x < iv[1]

def temporal_policy_checks(rng, counts):
    m = model({'v0': 'Vertex', 'm0': 'MetaVertex', 'e0': 'Edge'},
              [('e0', 'src', 'v0'), ('e0', 'tgt', 'm0')], [('e0', True)])
    base = encode(m)
    h = {s[1]: sid for sid, (s, p, o) in base.items() if p == 'type'}
    groups = {a: {h[a]} for a, _ in m.classes}
    for sid, (s, p, o) in base.items():
        if p != 'type':
            groups['e0'].add(sid)
    extra = {'membership': (h['v0'], 'inGraph', h['m0']),
             'attribute': (h['v0'], 'attr:k', ('literal', 'value')),
             'assessment': (('s', 'membership'), 'confidence', ('literal', '0.7')),
             'audit': (('s', 'assessment'), 'derivedFrom', ('literal', 'source'))}
    full = base | {('s', a): c for a, c in extra.items()}
    groups.update({a: {('s', a)} for a in extra})
    owner = {sid: c for c, ids in groups.items() for sid in ids}
    for policy in ('R', 'S'):
        done = 0
        while done < 1500:
            hist = rand_history(rng, containment=(policy == 'S'))
            if hist is None:
                continue
            L, J = hist
            for c in COMPONENTS:                                 # premises of the theorem
                for d in DEPS[c]:
                    assert L[d][0] <= L[c][0] and L[c][1] <= L[d][1]
                    if policy == 'S':
                        assert J[d][0] <= J[c][0] and J[c][1] <= J[d][1]
            for t, v in product(range(HORIZON), repeat=2):
                asserted = {c for c in COMPONENTS if live(L[c], t) and live(J[c], v)}
                # source side: least dependency-closed set
                ret = set(asserted)
                stack = list(ret)
                while stack:
                    for d in DEPS[stack.pop()]:
                        if d not in ret:
                            ret.add(d); stack.append(d)
                # encoded side: close over row references, then whole atom components
                rows = {sid for c in asserted for sid in groups[c]}
                stack = list(rows)
                while stack:
                    s, p, o = full[stack.pop()]
                    for x in (s, o):
                        if x in full and x not in rows:
                            rows.add(x); stack.append(x)
                comps = {owner[sid] for sid in rows}
                rows = {sid for c in comps for sid in groups[c]}
                assert comps == ret
                assert all(live(L[c], t) for c in ret)           # transaction time of retained referents
                if policy == 'S':
                    assert ret == asserted
                lhs = {sid: full[sid] for sid in rows}
                ranks(lhs)                                       # reference closed
                assert all(x in lhs for (s, p, o) in lhs.values() for x in (s, o) if x in full)
                roles = [r for r in m.roles if r[0] in ret]
                if 'membership' in ret:
                    roles.append(('v0', 'inGraph', 'm0'))
                chosen = model({a: k for a, k in m.classes if a in ret}, roles,
                               [f for f in m.flags if f[0] in ret],
                               [('v0', 'attr:k', 'value')] if 'attribute' in ret else [])
                rhs = encode(chosen)
                bearer = next((sid for sid, (_, p, _) in rhs.items() if p == 'inGraph'), None)
                if 'assessment' in ret:
                    rhs[('s', 'newassessment')] = (bearer, 'confidence', ('literal', '0.7'))
                if 'audit' in ret:
                    rhs[('s', 'newaudit')] = (('s', 'newassessment'), 'derivedFrom', ('literal', 'source'))
                def rooted(rs, sid):
                    s, p, o = rs[sid]
                    return (rooted(rs, s) if s in rs else s, p, rooted(rs, o) if o in rs else o)
                assert Counter(rooted(lhs, s) for s in lhs) == Counter(rooted(rhs, s) for s in rhs)
                structural = {sid: c for sid, c in lhs.items() if owner[sid] not in {'assessment', 'audit'}}
                assert encode(decode(structural, 'A')) == encode(chosen)
                counts['temporal_points_' + policy] += 1
                if ret != asserted:
                    counts['temporal_points_with_retained_' + policy] += 1
            counts['temporal_histories_' + policy] += 1
            done += 1
    # an annotation outside its bearer's valid interval exists under R and is illegal under S
    L = {c: (0, 4) for c in COMPONENTS}
    J = {c: (0, 4) for c in COMPONENTS}
    J['v0'] = J['m0'] = J['e0'] = J['membership'] = (0, 2)
    J['assessment'] = J['audit'] = (3, 4)
    assert all(live(L[c], 3) and live(J[c], 3) for c in ('assessment', 'audit'))
    assert not live(J['membership'], 3)
    assert not (J['assessment'][0] >= J['membership'][0] and J['assessment'][1] <= J['membership'][1])
    counts['annotation_outside_bearer'] += 1

def fragility_check(counts):
    c = {'a': 'Vertex', 'b': 'Vertex', 'e1': 'Edge', 'e2': 'Edge'}
    roles = [('e1', 'in', 'a'), ('e1', 'out', 'b'), ('e2', 'in', 'a'), ('e2', 'out', 'b')]
    J = {'e1': (0, 5), 'e2': (3, 8)}
    def snap(v):
        return model({k: x for k, x in c.items() if k in 'ab' or live(J[k], v)},
                     [r for r in roles if live(J[r[0]], v)])
    for v in range(8):
        validate(snap(v), 'D')                                # identified image: always fine
    for v in (3, 4):
        try:
            validate(snap(v), 'BB')
        except ValueError:
            continue
        raise AssertionError('extensionality should fail at v=%d' % v)
    for v in (0, 1, 2, 5, 6, 7):
        validate(snap(v), 'BB')
    counts['fragility_example'] += 1

# ---------------------------------------------------------------- version chains

def admissible(chains, L):
    """chains: list of lists of atoms, oldest first; L: atom -> (lo, hi)."""
    seen = set()
    for ch in chains:
        for a in ch:
            if a in seen:
                return False                                   # (V1)
            seen.add(a)
        for a, b in zip(ch, ch[1:]):
            if not L[a][1] <= L[b][0]:
                return False                                   # (V2)
    return True

def version_checks(rng, counts):
    for _ in range(3000):
        chains, L, k = [], {}, 0
        for _ in range(rng.randint(1, 4)):
            ch, t = [], rng.randint(0, 3)
            for _ in range(rng.randint(1, 4)):
                a = f'a{k}'; k += 1
                lo = t + rng.randint(0, 2)
                hi = lo + rng.randint(1, 3)
                L[a] = (lo, hi); t = hi
                ch.append(a)
            chains.append(ch)
        assert admissible(chains, L)
        chain_of = {a: i for i, ch in enumerate(chains) for a in ch}
        for t in range(0, 20):
            liveset = [a for a in L if live(L[a], t)]
            ids = [chain_of[a] for a in liveset]
            assert len(ids) == len(set(ids))                    # (i)+(ii)
        counts['version_chains'] += 1
        # mutations must break admissibility whenever lifetimes overlap
        if len(chains[0]) > 1:
            a, b = chains[0][0], chains[0][1]
            L2 = dict(L); L2[b] = (L[a][0], L[a][1])            # overlapping lifetimes
            assert not admissible(chains, L2)
            counts['version_violations_rejected'] += 1

# ---------------------------------------------------------------- driver

def run_extensions(counts):
    rng = random.Random(SEED)
    counts['necessity_witnesses'] = necessity_checks()
    randomized_checks(rng, counts)
    strata_and_cascade_checks(rng, counts)
    firing_checks(rng, counts)
    temporal_policy_checks(rng, counts)
    fragility_check(counts)
    version_checks(rng, counts)
    return {
        'necessity': 'one witness per constraint (18 constraints, 5 families); each rejected, accepted when only that constraint is dropped, still rejected when any other single constraint is dropped',
        'randomized': f'seed {SEED}; 1500 random models per family (D,HO,BB up to 8 vertices and 8 edges; U up to 6+6; A up to 3 atoms per class), 2 random mutations each',
        'strata_cascade': '1500 random NF4 stores (about 10-60 rows): rank partition, minimality chain, cascade rounds and fixpoint',
        'firing': 'exhaustive: 3 vertices, 0..2 edges, nonempty ends, all B,C,F; randomized: 4000 structures with 2..6 vertices and 1..4 edges with sequence oracle',
        'temporal': f'1500 random histories per policy over horizon {HORIZON}, every (t,v) point; dependency and clock premises asserted',
    }
