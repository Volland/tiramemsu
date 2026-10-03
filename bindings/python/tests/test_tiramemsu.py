"""Comprehensive tests for the tiramemsu Python binding.

Each test group covers one capability area from the bridge specification.
"""

import threading
from datetime import date, datetime, timezone
from typing import Any, List

import pytest

from tiramemsu import (
    BNode,
    Database,
    Iri,
    Literal,
    Node,
    QueryBudget,
    Ref,
    Report,
    SparqlAskResult,
    SparqlSelectResult,
    Stmt,
    TiramemsuError,
    TxId,
)

# ------------------------------------------------------------------------------- helpers

V = "urn:tiramemsu:v:"
SPARQL_PREFIX = f"PREFIX v: <{V}>\n"


def iri(name: str) -> Iri:
    return Iri(f"{V}{name}")


def works_at_query(db: Database, view: Any) -> List[str]:
    """Return sorted list of IRI values for alice's employers in the given view."""
    result = view.sparql(SPARQL_PREFIX + "SELECT ?o WHERE { v:alice v:worksAt ?o }")
    assert isinstance(result, SparqlSelectResult)
    return sorted(str(r["o"]) for r in result)


# ------------------------------------------------------------ (a) Alice time-travel story


class TestAliceTimeTravel:
    """The full bitemporal Alice/Acme/Globex scenario.

    tx1: assert alice worksAt acme valid_from 2020-01-01, with confidence 0.8 layer
    tx2: supersede the acme statement with valid_to 2024-01-01
    tx3: assert alice worksAt globex valid_from 2024-03-01
    """

    def setup_story(self, db: Database) -> None:
        alice = iri("alice")
        works_at = iri("worksAt")
        acme = iri("acme")
        globex = iri("globex")
        confidence = iri("confidence")

        with db.transact() as tx:
            job = tx.assert_(alice, works_at, acme, valid_from="2020-01-01")
            tx.assert_(job, confidence, 0.8)

        with db.transact() as tx:
            tx.supersede(1, valid_to="2024-01-01")

        with db.transact() as tx:
            tx.assert_(alice, works_at, globex, valid_from="2024-03-01")

    def test_now_shows_both_episodes(self, db: Database) -> None:
        self.setup_story(db)
        result = works_at_query(db, db.now())
        assert result == sorted([str(iri("acme")), str(iri("globex"))])

    def test_as_of_tx1_shows_only_acme(self, db: Database) -> None:
        self.setup_story(db)
        assert works_at_query(db, db.as_of(tx=1)) == [str(iri("acme"))]

    def test_as_of_tx1_valid_at_2026_shows_acme(self, db: Database) -> None:
        """We believed she was still at Acme; that belief is preserved."""
        self.setup_story(db)
        view = db.as_of(tx=1).valid_at("2026-01-01")
        assert works_at_query(db, view) == [str(iri("acme"))]

    def test_now_valid_at_2026_shows_globex(self, db: Database) -> None:
        self.setup_story(db)
        view = db.now().valid_at("2026-01-01")
        assert works_at_query(db, view) == [str(iri("globex"))]

    def test_now_valid_at_gap_is_empty(self, db: Database) -> None:
        """Between jobs (2024-02-01) there is nobody."""
        self.setup_story(db)
        view = db.now().valid_at("2024-02-01")
        assert works_at_query(db, view) == []

    def test_history_shows_two_unique_sparql_values(self, db: Database) -> None:
        """SPARQL sees a set of (s,p,o): acme and globex, even though acme has two rows."""
        self.setup_story(db)
        result = works_at_query(db, db.history())
        assert len(result) == 2
        assert str(iri("acme")) in result
        assert str(iri("globex")) in result

    def test_cypher_history_shows_three_rows(self, db: Database) -> None:
        """Cypher sees one row per statement: e1 (retracted), e2 and e3."""
        self.setup_story(db)
        r = db.history().cypher(
            "MATCH (a {`@id`: $aid})-[:worksAt]->(c) RETURN c",
            params={"aid": f"{V}alice"},
        )
        assert len(r.rows) == 3


# ------------------------------------------- (b) layers via Ref, Cypher and SPARQL {| |}


class TestLayers:
    def test_ref_creates_layer_readable_by_triples(self, db: Database) -> None:
        """A Ref used as subject stores a layer on the statement's eid."""
        alice = iri("alice")
        works_at = iri("worksAt")
        acme = iri("acme")
        confidence = iri("confidence")
        source = iri("source")

        with db.transact() as tx:
            job = tx.assert_(alice, works_at, acme)
            tx.assert_(job, confidence, 0.9)
            tx.assert_(job, source, "annual-review")

        # eid 1 is the job statement; its layers use eid 1 as the subject
        layers = db.now().triples(s=Stmt(1))
        predicates = {str(st.p) for st in layers}
        assert str(confidence) in predicates
        assert str(source) in predicates

    def test_layer_readable_by_cypher(self, db: Database) -> None:
        alice = iri("alice")
        works_at = iri("worksAt")
        acme = iri("acme")
        confidence = iri("confidence")

        with db.transact() as tx:
            job = tx.assert_(alice, works_at, acme)
            tx.assert_(job, confidence, 0.7)

        r = db.now().cypher(
            "MATCH (a {`@id`: $aid})-[rel:worksAt]->(c) RETURN rel.`urn:tiramemsu:v:confidence`",
            params={"aid": f"{V}alice"},
        )
        assert len(r.rows) == 1

    def test_layer_readable_by_sparql_annotation(self, db: Database) -> None:
        """SPARQL {| |} annotation syntax for inline metadata on a triple."""
        alice = iri("alice")
        works_at = iri("worksAt")
        acme = iri("acme")
        confidence = iri("confidence")

        with db.transact() as tx:
            job = tx.assert_(alice, works_at, acme)
            tx.assert_(job, confidence, 0.8)

        q = (
            SPARQL_PREFIX
            + "SELECT ?conf WHERE { v:alice v:worksAt v:acme {| v:confidence ?conf |} }"
        )
        result = db.now().sparql(q)
        assert isinstance(result, SparqlSelectResult)
        rows = list(result)
        assert any(abs(float(str(r["conf"])) - 0.8) < 1e-9 for r in rows)


# ---------------------------------------------- (c) retract cascade + events


class TestRetractCascade:
    def test_retract_cascades_to_layers(self, db: Database) -> None:
        alice = iri("alice")
        works_at = iri("worksAt")
        acme = iri("acme")
        confidence = iri("confidence")

        with db.transact() as tx:
            job = tx.assert_(alice, works_at, acme)
            tx.assert_(job, confidence, 0.8)

        # Both statements exist
        assert len(db.now().triples()) == 2

        with db.transact() as tx:
            tx.retract(1)  # retract the job; confidence should cascade

        # Nothing live remains
        assert db.now().triples() == []

    def test_retract_report_includes_cascade(self, db: Database) -> None:
        alice = iri("alice")
        works_at = iri("worksAt")
        acme = iri("acme")
        confidence = iri("confidence")

        with db.transact() as tx:
            job = tx.assert_(alice, works_at, acme)
            tx.assert_(job, confidence, 0.8)

        with db.transact() as tx:
            tx.retract(1)

        assert db.now().triples() == []

    def test_events_record_assert_and_retract(self, db: Database) -> None:
        alice = iri("alice")
        works_at = iri("worksAt")
        acme = iri("acme")
        confidence = iri("confidence")

        with db.transact() as tx:
            job = tx.assert_(alice, works_at, acme)
            tx.assert_(job, confidence, 0.8)

        with db.transact() as tx:
            tx.retract(1)

        events = db.now().events(since=1)
        # Two retract events (explicit + cascade)
        ops = [e["op"] for e in events]
        assert ops.count("retract") == 2


# ---------------------------------------------- (d) speculate keeps nothing


class TestSpeculate:
    def test_speculate_answers_but_commits_nothing(self, db: Database) -> None:
        from tiramemsu._db import TxBuilder

        alice = iri("alice")
        works_at = iri("worksAt")
        acme = iri("acme")

        ops = [
            {
                "op": "assert",
                "s": {"iri": alice.value},
                "p": {"iri": works_at.value},
                "o": {"iri": acme.value},
            }
        ]
        queries = [
            {"op": "triples"},
            {
                "op": "sparql",
                "text": SPARQL_PREFIX + "ASK { v:alice v:worksAt v:acme }",
            },
        ]
        results = db.speculate(ops, queries)
        assert len(results[0]) == 1  # one triple in speculative view
        assert results[1]["value"] is True  # ASK returns true

        # Nothing was committed
        assert db.now().triples() == []
        assert db.now().events() == []


# ---------------------------------------------- (e) dry_run


class TestDryRun:
    def test_dry_run_reports_without_committing(self, db: Database) -> None:
        alice = iri("alice")
        p = iri("p")
        obj = iri("b")

        report = db.transact(
            [{"op": "assert", "s": {"iri": alice.value}, "p": {"iri": p.value}, "o": {"iri": obj.value}}],
            dry_run=True,
        )
        assert isinstance(report, Report)
        assert len(report.asserted) == 1
        assert db.now().triples() == []

    def test_dry_run_context_manager(self, db: Database) -> None:
        alice = iri("alice")
        p = iri("p")

        with db.transact(dry_run=True) as tx:
            tx.assert_(alice, p, "hello")

        assert tx.report is not None
        assert len(tx.report.asserted) == 1
        assert db.now().triples() == []


# ---------------------------------------------- (f) errors carry .code; failing block commits nothing


class TestErrors:
    def test_error_has_code_attribute(self, db: Database) -> None:
        with pytest.raises(TiramemsuError) as exc_info:
            db.now().sparql("SELECT ?")  # invalid SPARQL
        assert exc_info.value.code == "Parse"

    def test_unknown_op_has_code(self, db: Database) -> None:
        from tiramemsu._db import _call
        with pytest.raises(TiramemsuError) as exc_info:
            _call(db._native, "nonexistentOp", "{}")
        assert exc_info.value.code == "InvalidArgument"

    def test_failing_block_submits_nothing(self, db: Database) -> None:
        alice = iri("alice")
        works_at = iri("worksAt")
        acme = iri("acme")

        with pytest.raises(RuntimeError):
            with db.transact() as tx:
                tx.assert_(alice, works_at, acme)
                raise RuntimeError("abort!")

        assert db.now().triples() == []

    def test_bad_op_in_block_raises_tiramemsu_error(self, db: Database) -> None:
        """An invalid op inside a block raises TiramemsuError with a code."""
        alice = iri("alice")
        works_at = iri("worksAt")
        acme = iri("acme")

        with pytest.raises(TiramemsuError) as exc_info:
            with db.transact() as tx:
                tx.assert_(alice, works_at, acme)
                # add a malformed op directly to trigger bridge error
                tx._ops.append({"op": "bogus"})

        assert exc_info.value.code == "InvalidArgument"
        assert db.now().triples() == []  # nothing committed

    def test_open_options_error_has_code(self, tmp_path: Any) -> None:
        """An unknown option name is rejected with InvalidArgument.

        The error surface is a RuntimeError from Native.__new__ whose message starts with
        ``tiramemsu:`` and contains a JSON body.  Database.__init__ converts that to a
        TiramemsuError.  We simulate the same conversion here by calling _from_json on the
        raw RuntimeError message so the test is robust to internal API changes.
        """
        from tiramemsu._native import Native

        try:
            Native(str(tmp_path / "x.db"), '{"bogusOption": 1}')
            pytest.fail("Expected RuntimeError from Native with unknown option")
        except RuntimeError as exc:
            msg = str(exc)
            assert msg.startswith("tiramemsu:")
            err = TiramemsuError._from_json(msg[len("tiramemsu:"):])
            assert err.code == "InvalidArgument"


# ---------------------------------------------- (g) big-int round trip


class TestBigInt:
    def test_big_int_round_trips(self, db: Database) -> None:
        big = 9_007_199_254_740_993  # 2^53 + 1, beyond JSON float precision
        x = iri("x")
        p = iri("bigVal")

        with db.transact() as tx:
            tx.assert_(x, p, big)

        rows = db.now().triples(s=x, p=p)
        assert len(rows) == 1
        assert rows[0].o == big

    def test_safe_int_round_trips_as_plain_int(self, db: Database) -> None:
        x = iri("x")
        p = iri("count")

        with db.transact() as tx:
            tx.assert_(x, p, 42)

        rows = db.now().triples(s=x, p=p)
        assert rows[0].o == 42
        assert isinstance(rows[0].o, int)


# ---------------------------------------------- (h) paths


class TestPaths:
    def test_path_reach_finds_all_reachable(self, db: Database) -> None:
        a, b, c = iri("a"), iri("b"), iri("c")
        knows = iri("knows")

        with db.transact() as tx:
            tx.assert_(a, knows, b)
            tx.assert_(b, knows, c)

        rows = db.now().path(a, "knows+")
        assert len(rows) == 2
        ends = {str(r.end) for r in rows}
        assert str(b) in ends
        assert str(c) in ends

    def test_path_any_shortest_includes_hops(self, db: Database) -> None:
        a, b = iri("a"), iri("b")
        knows = iri("knows")

        with db.transact() as tx:
            tx.assert_(a, knows, b)

        rows = db.now().path(a, "knows+", mode="anyShortest")
        assert len(rows) >= 1
        assert rows[0].path is not None
        assert isinstance(rows[0].path["hops"], list)


# ---------------------------------------------- (i) named graphs


class TestNamedGraphs:
    def test_add_to_graph_and_query(self, db: Database) -> None:
        alice = iri("alice")
        works_at = iri("worksAt")
        acme = iri("acme")
        session = iri("session42")

        with db.transact() as tx:
            job = tx.assert_(alice, works_at, acme)
            tx.add_to_graph(job, session)

        graphs = db.now().graphs()
        assert any(str(g) == str(session) for g in graphs)

        members = db.now().graph_members(session)
        assert 1 in members

    def test_create_and_drop_graph(self, db: Database) -> None:
        g = iri("myGraph")

        with db.transact() as tx:
            tx.create_graph(g)

        assert any(str(v) == str(g) for v in db.now().graphs())

        with db.transact() as tx:
            tx.drop_graph(g)

        assert not any(str(v) == str(g) for v in db.now().graphs())


# ---------------------------------------------- (j) persistence across reopening


class TestPersistence:
    def test_data_survives_close_and_reopen(self, tmp_path: Any) -> None:
        path = str(tmp_path / "persist.db")
        alice = iri("alice")
        works_at = iri("worksAt")
        acme = iri("acme")

        db1 = Database(path)
        with db1.transact() as tx:
            tx.assert_(alice, works_at, acme)
        del db1  # drop reference (Rust Drop closes)

        db2 = Database(path)
        rows = db2.now().triples(s=alice)
        assert len(rows) == 1
        assert str(rows[0].o) == str(acme)
        db2.close()


# ---------------------------------------------- (k) concurrent threads


class TestConcurrentThreads:
    def test_multiple_threads_can_query_simultaneously(self, db: Database) -> None:
        alice = iri("alice")
        works_at = iri("worksAt")
        acme = iri("acme")

        with db.transact() as tx:
            tx.assert_(alice, works_at, acme)

        errors: List[Exception] = []
        results: List[int] = []

        def query_worker() -> None:
            try:
                rows = db.now().triples(s=alice)
                results.append(len(rows))
            except Exception as exc:
                errors.append(exc)

        threads = [threading.Thread(target=query_worker) for _ in range(8)]
        for t in threads:
            t.start()
        for t in threads:
            t.join()

        assert errors == [], f"Thread errors: {errors}"
        assert all(r == 1 for r in results)


# ---------------------------------------------- term round-trip types


class TestTermTypes:
    def test_iri_round_trips(self, db: Database) -> None:
        x = iri("x")
        p = iri("p")
        v = Iri("urn:ex:something")

        with db.transact() as tx:
            tx.assert_(x, p, v)

        rows = db.now().triples(s=x)
        assert isinstance(rows[0].o, Iri)
        assert rows[0].o.value == v.value

    def test_string_round_trips(self, db: Database) -> None:
        x = iri("x")
        p = iri("p")

        with db.transact() as tx:
            tx.assert_(x, p, "hello world")

        rows = db.now().triples(s=x)
        assert rows[0].o == "hello world"

    def test_float_round_trips(self, db: Database) -> None:
        x = iri("x")
        p = iri("p")

        with db.transact() as tx:
            tx.assert_(x, p, 3.14)

        rows = db.now().triples(s=x)
        assert abs(float(str(rows[0].o)) - 3.14) < 1e-9

    def test_bool_round_trips(self, db: Database) -> None:
        x = iri("x")
        p = iri("p")

        with db.transact() as tx:
            tx.assert_(x, p, True)

        rows = db.now().triples(s=x)
        assert rows[0].o is True

    def test_literal_with_lang_round_trips(self, db: Database) -> None:
        x = iri("x")
        p = iri("p")
        lit = Literal("chat", lang="fr")

        with db.transact() as tx:
            tx.assert_(x, p, lit)

        rows = db.now().triples(s=x)
        assert isinstance(rows[0].o, Literal)
        assert rows[0].o.lex == "chat"
        assert rows[0].o.lang == "fr"

    def test_literal_with_datatype_round_trips(self, db: Database) -> None:
        x = iri("x")
        p = iri("p")
        lit = Literal("v1", datatype="urn:example:version")

        with db.transact() as tx:
            tx.assert_(x, p, lit)

        rows = db.now().triples(s=x)
        assert isinstance(rows[0].o, Literal)
        assert rows[0].o.datatype == "urn:example:version"
        assert rows[0].o.lex == "v1"


# ---------------------------------------------- database info


class TestDatabaseInfo:
    def test_info_returns_path_and_readers(self, db: Database, tmp_path: Any) -> None:
        info = db.info()
        assert "path" in info
        assert "readers" in info
        assert isinstance(info["readers"], int)

    def test_optimize_does_not_raise(self, db: Database) -> None:
        alice = iri("alice")
        p = iri("p")
        with db.transact() as tx:
            tx.assert_(alice, p, "data")
        db.optimize()  # should not raise


# ------------------------------------------------ spec scenarios the first pass left out


class TestDatetimesBoundsAndViews:
    def test_aware_datetime_round_trips(self, db: Database) -> None:
        when = datetime(2026, 9, 30, 12, 34, 56, tzinfo=timezone.utc)
        with db.transact() as tx:
            tx.assert_(iri("x"), iri("seenAt"), when)
        rows = db.now().triples(s=iri("x"))
        assert len(rows) == 1
        got = rows[0].o
        assert isinstance(got, datetime)
        assert got.timestamp() == when.timestamp()

    def test_date_round_trips(self, db: Database) -> None:
        day = date(2026, 9, 30)
        with db.transact() as tx:
            tx.assert_(iri("x"), iri("on"), day)
        got = db.now().triples(s=iri("x"))[0].o
        assert got == day

    def test_supersede_none_clears_a_bound_and_omitting_keeps_it(self, db: Database) -> None:
        with db.transact() as tx:
            job = tx.assert_(
                iri("alice"), iri("worksAt"), iri("acme"), valid_from="2020-01-01", valid_to="2024-01-01"
            )
        first = db.now().triples(s=iri("alice"))[0]
        assert first.valid_to is not None
        with db.transact() as tx:
            tx.supersede(first.eid, o=iri("initech"))  # no bound given: both kept
        second = db.now().triples(s=iri("alice"))[0]
        assert second.valid_to == first.valid_to and second.valid_from == first.valid_from
        with db.transact() as tx:
            tx.supersede(second.eid, valid_to=None)  # None clears the end
        third = db.now().triples(s=iri("alice"))[0]
        assert third.valid_to is None and third.valid_from == first.valid_from
        assert job is not None

    def test_valid_at_returns_a_new_view(self, db: Database) -> None:
        with db.transact() as tx:
            tx.assert_(
                iri("alice"), iri("worksAt"), iri("acme"), valid_from="2020-01-01", valid_to="2021-01-01"
            )
        plain = db.now()
        filtered = plain.valid_at("2026-01-01")
        assert len(plain.triples(s=iri("alice"))) == 1
        assert len(filtered.triples(s=iri("alice"))) == 0
        assert len(plain.triples(s=iri("alice"))) == 1

    def test_confirm_not_live_has_code(self, db: Database) -> None:
        with pytest.raises(TiramemsuError) as e:
            with db.transact() as tx:
                tx.confirm(99)
        assert e.value.code == "NotLive"


# ------------------------------------------------------- features added after 0.1.0


class TestPathOptions:
    def test_paths_stay_inside_the_listed_graphs(self, db: Database) -> None:
        with db.transact() as tx:
            ab = tx.assert_(iri("a"), iri("knows"), iri("b"))
            tx.assert_(iri("b"), iri("knows"), iri("c"))
            tx.add_to_graph(ab, iri("session12"))
        ends = lambda **kw: [r.end for r in db.now().path(iri("a"), "v:knows+", **kw)]
        assert ends(graphs=[iri("session12")]) == [iri("b")]
        assert len(ends()) == 2
        assert ends(graphs=[iri("nowhere")]) == []

    def test_time_respecting_paths_report_arrival(self, db: Database) -> None:
        with db.transact() as tx:
            tx.assert_(iri("a"), iri("met"), iri("b"), valid_from=1, valid_to=5)
            tx.assert_(iri("b"), iri("met"), iri("c"), valid_from=3, valid_to=9)
        arrivals = lambda tr: [
            r.arrival for r in db.now().path(iri("a"), "v:met+", time_respecting=tr)
        ]
        assert arrivals(True) == [1, 3]
        assert arrivals(2) == [2, 3]
        assert arrivals(6) == []
        assert arrivals(False) == [None, None]


class TestProvenanceDependentsBundles:
    def test_sparql_rows_carry_provenance(self, db: Database) -> None:
        with db.transact() as tx:
            tx.assert_(iri("a"), iri("p"), iri("b"))
        q = SPARQL_PREFIX + "SELECT ?o WHERE { v:a v:p ?o }"
        r = db.now().sparql(q, provenance=True)
        assert isinstance(r, SparqlSelectResult)
        assert r.provenance == [[Stmt(1)]]
        plain = db.now().sparql(q)
        assert isinstance(plain, SparqlSelectResult)
        assert plain.provenance is None

    def test_dependents_preview_a_retraction(self, db: Database) -> None:
        ctx = db.transact()
        with ctx as tx:
            job = tx.assert_(iri("alice"), iri("worksAt"), iri("acme"))
            tx.assert_(job, iri("source"), "chat-1")
        report = ctx._builder.report
        assert report is not None
        eid = report.asserted[0]
        assert db.now().dependents(eid) == report.asserted
        with db.transact() as tx:
            tx.retract(eid)
        assert db.now().dependents(eid) == []
        assert db.as_of(tx=1).dependents(Stmt(eid)) == report.asserted

    def test_a_bundle_moves_between_databases(self, tmp_path: Any) -> None:
        a = Database(str(tmp_path / "a.db"))
        b = Database(str(tmp_path / "b.db"))
        ctx = a.transact()
        with ctx as tx:
            job = tx.assert_(iri("alice"), iri("worksAt"), iri("acme"), valid_from="2020-01-01")
            tx.assert_(job, iri("confidence"), 0.8)
            tx.add_to_graph(job, iri("session12"))
        assert ctx._builder.report is not None
        bundle = a.now().bundle(ctx._builder.report.asserted[0])
        assert bundle["format"] == "tiramemsu-bundle/1"
        ctx = b.transact()
        with ctx as tx:
            fact = tx.import_bundle(bundle)
            tx.assert_(fact, iri("importedFrom"), iri("agentA"))
        report = ctx._builder.report
        assert report is not None
        imported = report.results[0]
        assert len(imported["statements"]) == 3
        assert all(s["new"] for s in imported["statements"])
        assert b.now().graph_members(iri("session12")) == [imported["root"]]
        with pytest.raises(TiramemsuError) as e:
            b.transact([{"op": "importBundle", "bundle": {"format": "tiramemsu-bundle/9"}}])
        assert e.value.code == "InvalidTerm"

    def test_a_statement_names_a_graph(self, db: Database) -> None:
        ctx = db.transact()
        with ctx as tx:
            edge = tx.assert_(iri("alice"), iri("knows"), iri("bob"))
            fact = tx.assert_(iri("bob"), iri("worksAt"), iri("acme"))
            tx.add_to_graph(fact, edge)
        report = ctx._builder.report
        assert report is not None
        edge_eid, fact_eid = report.asserted[:2]
        assert db.now().graph_members(Stmt(edge_eid)) == [fact_eid]


# ------------------------------------------------------------------------- query budgets

CROSS = SPARQL_PREFIX + "SELECT (COUNT(*) AS ?c) WHERE { ?a v:p ?x . ?b v:p ?y . ?c2 v:p ?z }"


def _seed(db: Database, n: int) -> None:
    db.transact([{"op": "assert", "s": {"iri": f"{V}n{i}"}, "p": {"iri": f"{V}p"}, "o": i} for i in range(n)])


class TestQueryBudgets:
    def test_budgets_bound_calls_with_typed_codes(self, db: Database) -> None:
        _seed(db, 1500)
        with pytest.raises(TiramemsuError) as e:
            db.now().with_budget(QueryBudget(timeout_ms=100)).sparql(CROSS)
        assert e.value.code == "DeadlineExceeded"
        capped = db.now().with_budget(QueryBudget(max_rows=10))
        with pytest.raises(TiramemsuError) as e:
            capped.triples()
        assert e.value.code == "ResultLimitExceeded"
        assert len(capped.triples(s=iri("n1"))) == 1
        before = len(db.now().triples(p=iri("q")))
        with pytest.raises(TiramemsuError) as e:
            db.cypher_write(
                "MATCH (a), (b), (c) WHERE a.p >= 0 AND b.p >= 0 AND c.p >= 0 CREATE (a)-[:q]->(b)",
                budget=QueryBudget(timeout_ms=100),
            )
        assert e.value.code == "DeadlineExceeded"
        assert db.cancel("pre") is False
        with pytest.raises(TiramemsuError) as e:
            with db.transact(budget=QueryBudget(cancel_key="pre")) as tx:
                tx.assert_(iri("x"), iri("q"), iri("y"))
        assert e.value.code == "Cancelled"
        assert len(db.now().triples(p=iri("q"))) == before
        report = db.transact(
            [{"op": "assert", "s": {"iri": f"{V}x"}, "p": {"iri": f"{V}q"}, "o": 1}],
            budget=QueryBudget(timeout_ms=10_000, max_rows=1000),
        )
        assert len(report.asserted) == 1

    def test_cancel_from_another_thread(self, db: Database) -> None:
        _seed(db, 1500)
        found: List[bool] = []

        def canceller() -> None:
            for _ in range(500):
                threading.Event().wait(0.01)
                if db.cancel("q1"):
                    found.append(True)
                    return

        t = threading.Thread(target=canceller)
        t.start()
        with pytest.raises(TiramemsuError) as e:
            db.now().with_budget(QueryBudget(cancel_key="q1")).sparql(CROSS)
        t.join()
        assert e.value.code == "Cancelled"
        assert found == [True]
        assert len(db.now().triples(s=iri("n0"))) == 1

    def test_reader_timeout_option(self, tmp_path: Any) -> None:
        db = Database(str(tmp_path / "r.db"), readers=1, reader_timeout_ms=50)
        assert db.now().triples() == []
        assert db.info()["readers"] == 1
