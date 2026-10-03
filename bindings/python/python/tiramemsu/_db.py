"""The public ``Database`` and ``View`` classes for tiramemsu, plus the transaction API.

All communication with the native layer goes through :class:`~tiramemsu._native.Native`
via JSON strings.  The module converts Python values to/from the bridge JSON forms
defined in :mod:`tiramemsu._types`.
"""

from __future__ import annotations

import json
from typing import Any, Dict, Iterator, List, Optional, Union, overload

from ._native import Native
from ._types import (
    CypherResult,
    CypherWriteResult,
    ImportProgress,
    ImportSummary,
    PathRow,
    QueryBudget,
    Report,
    SparqlResult,
    Statement,
    Stmt,
    TiramemsuError,
    import_progress_from_json,
    import_summary_from_json,
    report_from_json,
    sparql_result_from_json,
    statement_from_json,
    term_from_json,
    text_hit_from_json,
    TextHit,
    term_to_json,
    time_to_json,
)

# A sentinel that is distinct from None (which means "clear the bound").
_UNSET: Any = object()


# --------------------------------------------------------------------------- native call


def _call(native: Native, op: str, args: str) -> str:
    """Call the native layer, converting the ``RuntimeError`` prefix to ``TiramemsuError``."""
    try:
        return native.call(op, args)
    except RuntimeError as exc:
        msg = str(exc)
        if msg.startswith("tiramemsu:"):
            raise TiramemsuError._from_json(msg[len("tiramemsu:") :]) from exc
        raise


def _eid_json(v: Any) -> Any:
    """A statement id (``int`` or :class:`Stmt`) as bridge JSON."""
    if isinstance(v, Stmt):
        return {"stmt": v.eid}
    if isinstance(v, int) and not isinstance(v, bool):
        return v
    raise TypeError(f"eid must be int or Stmt; got {type(v).__name__!r}")


# ------------------------------------------------------------------------------- Ref


class Ref:
    """A handle to a named result inside a pending :class:`TxBuilder` context.

    Returned by builder methods that create statements.  Use it wherever a term or eid
    is accepted in later operations of the same transaction.
    """

    def __init__(self, name: str) -> None:
        self._name = name

    def _to_json(self) -> Any:
        return {"ref": self._name}

    def __repr__(self) -> str:
        return f"Ref({self._name!r})"


# ---------------------------------------------------------------------------- TxBuilder


class TxBuilder:
    """Records transaction operations for a ``with db.transact() as tx:`` block.

    Methods append to an internal op list; the list is submitted atomically when the
    ``with`` block exits cleanly.  Methods that create statements return a :class:`Ref`
    usable in later calls.

    After a clean exit, :attr:`report` holds the :class:`Report` of the committed
    transaction.
    """

    def __init__(self) -> None:
        self._ops: List[Dict[str, Any]] = []
        self.report: Optional[Report] = None
        self._counter = 0

    def _fresh(self) -> str:
        self._counter += 1
        return f"_r{self._counter}"

    def _v(self, v: Any) -> Any:
        """Term or Ref to bridge JSON."""
        if isinstance(v, Ref):
            return v._to_json()
        return term_to_json(v)

    def _eid(self, v: Any) -> Any:
        """Eid value (int, Stmt, or Ref) to bridge JSON."""
        if isinstance(v, Ref):
            return v._to_json()
        from ._types import Stmt  # local to avoid re-export confusion

        if isinstance(v, Stmt):
            return {"stmt": v.eid}
        if isinstance(v, int):
            return v
        raise TypeError(f"eid must be int, Stmt or Ref; got {type(v).__name__!r}")

    # ----------------------------------------------------------------- write ops

    def assert_(
        self,
        s: Any,
        p: Any,
        o: Any,
        *,
        valid_from: Any = None,
        valid_to: Any = None,
        on_existing: str = "return",
    ) -> Ref:
        """Assert ``(s, p, o)`` with an optional valid interval.

        Returns a :class:`Ref` to the created or existing statement's eid.
        """
        name = self._fresh()
        op: Dict[str, Any] = {
            "op": "assert",
            "s": self._v(s),
            "p": self._v(p),
            "o": self._v(o),
            "as": name,
            "onExisting": on_existing,
        }
        if valid_from is not None:
            op["validFrom"] = time_to_json(valid_from)
        if valid_to is not None:
            op["validTo"] = time_to_json(valid_to)
        self._ops.append(op)
        return Ref(name)

    def create(
        self,
        s: Any,
        p: Any,
        o: Any,
        *,
        valid_from: Any = None,
        valid_to: Any = None,
    ) -> Ref:
        """Create a new statement unconditionally.  Returns a :class:`Ref`."""
        name = self._fresh()
        op: Dict[str, Any] = {
            "op": "create",
            "s": self._v(s),
            "p": self._v(p),
            "o": self._v(o),
            "as": name,
        }
        if valid_from is not None:
            op["validFrom"] = time_to_json(valid_from)
        if valid_to is not None:
            op["validTo"] = time_to_json(valid_to)
        self._ops.append(op)
        return Ref(name)

    def retract(self, eid: Any) -> None:
        """Retract the statement with this eid (and cascade to any layers on it)."""
        self._ops.append({"op": "retract", "eid": self._eid(eid)})

    def retract_matching(
        self,
        s: Any = None,
        p: Any = None,
        o: Any = None,
    ) -> None:
        """Retract all statements matching the given pattern (``None`` is wildcard)."""
        op: Dict[str, Any] = {"op": "retractMatching"}
        if s is not None:
            op["s"] = self._v(s)
        if p is not None:
            op["p"] = self._v(p)
        if o is not None:
            op["o"] = self._v(o)
        self._ops.append(op)

    def supersede(
        self,
        eid: Any,
        *,
        o: Any = _UNSET,
        valid_from: Any = _UNSET,
        valid_to: Any = _UNSET,
    ) -> Ref:
        """Supersede a live statement, changing its value or valid interval.

        Omit a keyword to leave that field unchanged.  Pass ``None`` to clear a bound.
        Returns a :class:`Ref` to the new statement's eid.
        """
        name = self._fresh()
        patch: Dict[str, Any] = {}
        if o is not _UNSET:
            patch["o"] = self._v(o)
        if valid_from is not _UNSET:
            patch["validFrom"] = None if valid_from is None else time_to_json(valid_from)
        if valid_to is not _UNSET:
            patch["validTo"] = None if valid_to is None else time_to_json(valid_to)
        self._ops.append(
            {"op": "supersede", "eid": self._eid(eid), "patch": patch, "as": name}
        )
        return Ref(name)

    def confirm(self, eid: Any) -> None:
        """Record that another source agrees with a live statement (``sys:confirmedBy``); nothing else changes."""
        self._ops.append({"op": "confirm", "eid": self._eid(eid)})

    def meta(self, p: Any, o: Any) -> Ref:
        """Assert a database-level metadata triple.  Returns a :class:`Ref`."""
        name = self._fresh()
        self._ops.append({"op": "meta", "p": self._v(p), "o": self._v(o), "as": name})
        return Ref(name)

    def upsert(self, p: Any, o: Any) -> None:
        """Create or return the unique node for ``(p, o)``."""
        self._ops.append({"op": "upsert", "p": self._v(p), "o": self._v(o)})

    def new_node(self) -> None:
        """Allocate a fresh anonymous node."""
        self._ops.append({"op": "newNode"})

    def add_to_graph(self, eid: Any, graph: Any, *, on_existing: str = "return") -> Ref:
        """Tag statement *eid* as a member of *graph*.  Returns a :class:`Ref`."""
        name = self._fresh()
        self._ops.append(
            {
                "op": "addToGraph",
                "eid": self._eid(eid),
                "graph": self._v(graph),
                "as": name,
                "onExisting": on_existing,
            }
        )
        return Ref(name)

    def remove_from_graph(self, eid: Any, graph: Any) -> None:
        """Remove statement *eid* from *graph*."""
        self._ops.append(
            {"op": "removeFromGraph", "eid": self._eid(eid), "graph": self._v(graph)}
        )

    def clear_graph(self, graph: Any) -> None:
        """Remove all members from *graph*."""
        self._ops.append({"op": "clearGraph", "graph": self._v(graph)})

    def create_graph(self, graph: Any) -> Ref:
        """Create a named graph.  Returns a :class:`Ref`."""
        name = self._fresh()
        self._ops.append({"op": "createGraph", "graph": self._v(graph), "as": name})
        return Ref(name)

    def drop_graph(self, graph: Any) -> None:
        """Drop a named graph and all its membership statements."""
        self._ops.append({"op": "dropGraph", "graph": self._v(graph)})

    def import_bundle(self, bundle: Dict[str, Any]) -> Ref:
        """Import a bundle read with :meth:`View.bundle`, possibly from another database.

        Returns a :class:`Ref` to the imported root statement; the op's entry in
        ``report.results`` is ``{"root", "statements": [{"id", "eid", "new"}]}``.
        """
        name = self._fresh()
        self._ops.append({"op": "importBundle", "bundle": bundle, "as": name})
        return Ref(name)

    def cypher(self, text: str, params: Optional[Dict[str, Any]] = None) -> None:
        """Run a Cypher write statement as part of this transaction."""
        op: Dict[str, Any] = {"op": "cypher", "text": text}
        if params:
            op["params"] = params
        self._ops.append(op)


# ---------------------------------------------------------------------------- TxContext


class TxContext:
    """Context manager that submits a :class:`TxBuilder`'s ops on clean exit."""

    def __init__(
        self,
        native: Native,
        *,
        dry_run: bool = False,
        max_cascade: Optional[int] = None,
        budget: Optional[QueryBudget] = None,
        op: str = "transact",
        extra: Optional[Dict[str, Any]] = None,
    ) -> None:
        self._native = native
        self._dry_run = dry_run
        self._max_cascade = max_cascade
        self._budget = budget
        self._op = op
        self._extra = extra or {}
        self._builder = TxBuilder()

    def __enter__(self) -> TxBuilder:
        return self._builder

    def __exit__(
        self,
        exc_type: Any,
        exc_val: Any,
        exc_tb: Any,
    ) -> None:
        if exc_type is not None:
            return  # don't submit when the block raised
        args = _tx_args(
            self._builder._ops, self._dry_run, self._max_cascade, self._budget
        )
        args.update(self._extra)
        result_text = _call(self._native, self._op, json.dumps(args))
        self._builder.report = report_from_json(json.loads(result_text))


def _tx_args(
    ops: List[Any],
    dry_run: bool,
    max_cascade: Optional[int],
    budget: Optional[QueryBudget],
) -> Dict[str, Any]:
    """The bridge arguments of a transaction (or import chunk) of *ops*."""
    args: Dict[str, Any] = {"ops": ops}
    options: Dict[str, Any] = {}
    if dry_run:
        options["dryRun"] = True
    if max_cascade is not None:
        options["maxCascade"] = max_cascade
    if options:
        args["options"] = options
    if budget is not None:
        args["budget"] = budget._to_json()
    return args


# ------------------------------------------------------------------------ BulkImport


class BulkImport:
    """A bulk import session from :meth:`Database.bulk_import`.

    Each chunk is one atomic transaction; planner statistics are refreshed once by
    :meth:`finish`. While the session is open other writes fail with
    ``ImportInProgress`` and reads see the last committed chunk. Used as a context
    manager it finishes on a clean exit and cancels when the block raises; the
    summary is on :attr:`summary` afterwards.
    """

    def __init__(self, native: Native, session: int) -> None:
        self._native = native
        self.session = session
        self.summary: Optional[ImportSummary] = None

    @overload
    def chunk(
        self,
        ops: List[Any],
        *,
        dry_run: bool = ...,
        max_cascade: Optional[int] = ...,
        budget: Optional[QueryBudget] = ...,
    ) -> Report: ...

    @overload
    def chunk(
        self,
        ops: None = None,
        *,
        dry_run: bool = ...,
        max_cascade: Optional[int] = ...,
        budget: Optional[QueryBudget] = ...,
    ) -> TxContext: ...

    def chunk(
        self,
        ops: Optional[List[Any]] = None,
        *,
        dry_run: bool = False,
        max_cascade: Optional[int] = None,
        budget: Optional[QueryBudget] = None,
    ) -> Union[Report, TxContext]:
        """Commit one chunk, in the two forms of :meth:`Database.transact`.

        A failing chunk raises and rolls back alone; earlier chunks stay committed.
        """
        extra = {"session": self.session}
        if ops is not None:
            args = _tx_args(ops, dry_run, max_cascade, budget)
            args.update(extra)
            result_text = _call(self._native, "importChunk", json.dumps(args))
            return report_from_json(json.loads(result_text))
        return TxContext(
            self._native,
            dry_run=dry_run,
            max_cascade=max_cascade,
            budget=budget,
            op="importChunk",
            extra=extra,
        )

    def progress(self) -> ImportProgress:
        """The progress so far."""
        args = json.dumps({"session": self.session})
        return import_progress_from_json(
            json.loads(_call(self._native, "importProgress", args))
        )

    def finish(self) -> ImportSummary:
        """Run one full analysis, release the write lease and return the summary."""
        args = json.dumps({"session": self.session})
        self.summary = import_summary_from_json(
            json.loads(_call(self._native, "importFinish", args))
        )
        return self.summary

    def cancel(self) -> ImportProgress:
        """End the session without analysis; committed chunks stay, statistics are due."""
        args = json.dumps({"session": self.session})
        return import_progress_from_json(
            json.loads(_call(self._native, "importCancel", args))
        )

    def __enter__(self) -> "BulkImport":
        return self

    def __exit__(self, exc_type: Any, exc_val: Any, exc_tb: Any) -> None:
        if exc_type is None:
            self.finish()
        else:
            self.cancel()


# ------------------------------------------------------------------------------- View


class View:
    """An immutable view of the database at a specific transaction and/or valid time.

    Obtain one from :meth:`Database.now`, :meth:`Database.as_of` or
    :meth:`Database.history`, then optionally narrow with :meth:`valid_at`.
    """

    def __init__(
        self,
        native: Native,
        view_json: Dict[str, Any],
        budget: Optional[QueryBudget] = None,
    ) -> None:
        self._native = native
        self._view = view_json
        self._budget = budget

    def valid_at(self, when: Any) -> "View":
        """Return a view further restricted to facts valid at *when*.

        *when* may be a ``datetime``, ``date``, epoch-ms ``int``, or RFC 3339 ``str``.
        """
        new_view = {**self._view, "validAt": time_to_json(when)}
        return View(self._native, new_view, self._budget)

    def with_budget(self, budget: QueryBudget) -> "View":
        """Return the same view with every call through it bounded by *budget*.

        Each call (``sparql``, ``cypher``, ``triples``, ``path``, ...) is one operation
        with its own deadline and its own row and byte budget.
        """
        return View(self._native, self._view, budget)

    def _call(self, op: str, extra: Dict[str, Any]) -> str:
        args = {**extra, "view": self._view}
        if self._budget is not None:
            args["budget"] = self._budget._to_json()
        return _call(self._native, op, json.dumps(args))

    def sparql(self, text: str, *, provenance: bool = False) -> SparqlResult:
        """Run a SPARQL query on this view.

        Returns :class:`~._types.SparqlSelectResult`,
        :class:`~._types.SparqlAskResult`, :class:`~._types.SparqlGraphResult`, or
        :class:`~._types.SparqlUpdateResult` depending on the query form.  With
        *provenance*, a SELECT result also lists the statements behind each row.
        """
        args: Dict[str, Any] = {"text": text}
        if provenance:
            args["provenance"] = True
        result_text = self._call("sparql", args)
        return sparql_result_from_json(json.loads(result_text))

    def cypher(
        self,
        text: str,
        params: Optional[Dict[str, Any]] = None,
    ) -> CypherResult:
        """Run a read-only Cypher query on this view."""
        args: Dict[str, Any] = {"text": text}
        if params:
            args["params"] = params
        result_text = self._call("cypher", args)
        j = json.loads(result_text)
        return CypherResult(
            columns=list(j.get("columns") or []),
            rows=list(j.get("rows") or []),
        )

    def triples(
        self,
        s: Any = None,
        p: Any = None,
        o: Any = None,
    ) -> List[Statement]:
        """Return statements matching the given pattern (``None`` is wildcard)."""
        args: Dict[str, Any] = {}
        if s is not None:
            args["s"] = term_to_json(s)
        if p is not None:
            args["p"] = term_to_json(p)
        if o is not None:
            args["o"] = term_to_json(o)
        rows = json.loads(self._call("triples", args))
        return [statement_from_json(r) for r in rows]

    def path(
        self,
        start: Any,
        path_expr: str,
        *,
        mode: str = "reach",
        max_hops: Optional[int] = None,
        graphs: Optional[List[Any]] = None,
        time_respecting: Any = False,
    ) -> List[PathRow]:
        """Find all nodes reachable from *start* via *path_expr*.

        *mode* is ``"reach"`` (default), ``"trail"``, ``"anyShortest"`` or
        ``"allShortest"``.  *graphs* keeps every hop inside the listed graphs (a term
        that is not stored names no graph).  *time_respecting* is ``True`` or a time
        to start after: each hop must start no earlier than the previous one, and
        every row carries its ``arrival``.
        """
        args: Dict[str, Any] = {
            "start": term_to_json(start),
            "path": path_expr,
            "mode": mode,
        }
        if max_hops is not None:
            args["maxHops"] = max_hops
        if graphs is not None:
            args["graphs"] = [term_to_json(g) for g in graphs]
        if time_respecting is True:
            args["timeRespecting"] = True
        elif time_respecting is not False and time_respecting is not None:
            args["timeRespecting"] = {"after": time_to_json(time_respecting)}
        rows: List[Any] = json.loads(self._call("path", args))
        return [
            PathRow(
                start=term_from_json(r["start"]),
                end=term_from_json(r["end"]),
                hops=int(r["hops"]),
                path=r.get("path") if r.get("path") is not None else None,
                arrival=None if r.get("arrival") is None else int(r["arrival"]),
            )
            for r in rows
        ]

    def events(self, since: int = 0) -> List[Dict[str, Any]]:
        """Return events (asserts / retracts) with ``t > since``."""
        rows: List[Any] = json.loads(self._call("events", {"since": since}))
        return list(rows)

    def graphs(self) -> List[Any]:
        """Return the named graphs present in this view."""
        items: List[Any] = json.loads(self._call("graphs", {}))
        return [term_from_json(g) for g in items]

    def graph_members(self, graph: Any) -> List[int]:
        """Return the eids of statements that belong to *graph*."""
        items: List[Any] = json.loads(
            self._call("graphMembers", {"graph": term_to_json(graph)})
        )
        return [int(e) for e in items]

    def dependents(self, eid: Any) -> List[int]:
        """Return the statements that stand on *eid* (its layers and memberships,
        transitively): what retracting it would cascade to."""
        items: List[Any] = json.loads(self._call("dependents", {"eid": _eid_json(eid)}))
        return [int(e) for e in items]

    def bundle(self, eid: Any) -> Dict[str, Any]:
        """Return statement *eid* with its layers and evidence as ``tiramemsu-bundle/1``
        JSON, for :meth:`TxBuilder.import_bundle`."""
        return dict(json.loads(self._call("bundle", {"eid": _eid_json(eid)})))

    def text_search(
        self,
        text: str,
        *,
        mode: str = "all",
        graphs: Optional[List[Any]] = None,
        predicates: Optional[List[Any]] = None,
        limit: Optional[int] = None,
        confidence: Any = None,
    ) -> List["TextHit"]:
        """Text recall: the statements of this view whose string object matches
        *text*, ranked by lexical score, then confidence, confirmations, authors and
        recency, then eid.

        *mode* is ``"all"`` (every word, default), ``"any"`` or ``"phrase"``.
        *graphs* and *predicates* filter the statements, *limit* keeps the first hits
        by rank, and *confidence* names the confidence predicate (default
        ``v:confidence``).  Needs the text index (``Database(..., text_index=True)``
        or :meth:`Database.rebuild_text_index`): raises ``TextIndexUnavailable``
        without it and ``MissingCapability`` on a SQLite without FTS5.
        """
        args: Dict[str, Any] = {"text": text, "mode": mode}
        if graphs is not None:
            args["graphs"] = [term_to_json(g) for g in graphs]
        if predicates is not None:
            args["predicates"] = [term_to_json(p) for p in predicates]
        if limit is not None:
            args["limit"] = limit
        if confidence is not None:
            args["confidence"] = term_to_json(confidence)
        rows: List[Any] = json.loads(self._call("textSearch", args))
        return [text_hit_from_json(r) for r in rows]

    def values(self, s: Any, key: Any) -> List[Any]:
        """Return all objects ``o`` where ``(s, key, o)`` exists in this view."""
        items: List[Any] = json.loads(
            self._call("values", {"s": term_to_json(s), "key": term_to_json(key)})
        )
        return [term_from_json(v) for v in items]


# --------------------------------------------------------------------------- Database


class Database:
    """A tiramemsu database.

    Opens (or creates) the database file at *path*.  All keyword arguments are optional
    open options passed to the engine.

    Can be used as a context manager; ``close`` is a no-op (the database is closed when
    the object is garbage-collected by Rust's ``Drop`` impl).
    """

    def __init__(
        self,
        path: str,
        *,
        readers: Optional[int] = None,
        busy_timeout_ms: Optional[int] = None,
        term_cache_capacity: Optional[int] = None,
        optimize_every: Optional[int] = None,
        path_max_hops: Optional[int] = None,
        path_max_states: Optional[int] = None,
        reader_timeout_ms: Optional[int] = None,
        text_index: Optional[bool] = None,
    ) -> None:
        options: Dict[str, Any] = {}
        if readers is not None:
            options["readers"] = readers
        if busy_timeout_ms is not None:
            options["busyTimeoutMs"] = busy_timeout_ms
        if term_cache_capacity is not None:
            options["termCacheCapacity"] = term_cache_capacity
        if optimize_every is not None:
            options["optimizeEvery"] = optimize_every
        if path_max_hops is not None:
            options["pathMaxHops"] = path_max_hops
        if path_max_states is not None:
            options["pathMaxStates"] = path_max_states
        if reader_timeout_ms is not None:
            options["readerTimeoutMs"] = reader_timeout_ms
        if text_index is not None:
            options["textIndex"] = bool(text_index)
        opts_str: Optional[str] = json.dumps(options) if options else None
        try:
            self._native = Native(path, opts_str)
        except RuntimeError as exc:
            msg = str(exc)
            if msg.startswith("tiramemsu:"):
                raise TiramemsuError._from_json(msg[len("tiramemsu:") :]) from exc
            raise

    # ----------------------------------------------------------------- context manager

    def close(self) -> None:
        """No-op: the database is closed when the Python object is garbage-collected."""

    def __enter__(self) -> "Database":
        return self

    def __exit__(self, *_: Any) -> None:
        self.close()

    # ---------------------------------------------------------------------- views

    def now(self) -> View:
        """A view of all currently-live statements (default view)."""
        return View(self._native, {"kind": "now"})

    def as_of(
        self,
        *,
        tx: Optional[int] = None,
        instant: Any = None,
    ) -> View:
        """A view of the database as it was at a past transaction or wall-clock time.

        Specify exactly one of *tx* (a transaction number) or *instant* (a
        ``datetime``, epoch-ms ``int``, or RFC 3339 ``str``).
        """
        if (tx is None) == (instant is None):
            raise ValueError("specify exactly one of tx and instant")
        view: Dict[str, Any] = {"kind": "asOf"}
        if tx is not None:
            view["tx"] = tx
        else:
            view["instant"] = time_to_json(instant)
        return View(self._native, view)

    def history(self) -> View:
        """A view that includes all statements ever asserted, including retracted ones."""
        return View(self._native, {"kind": "history"})

    # ------------------------------------------------------------------- writes

    @overload
    def transact(
        self,
        ops: List[Any],
        *,
        dry_run: bool = ...,
        max_cascade: Optional[int] = ...,
        budget: Optional[QueryBudget] = ...,
    ) -> Report: ...

    @overload
    def transact(
        self,
        ops: None = None,
        *,
        dry_run: bool = ...,
        max_cascade: Optional[int] = ...,
        budget: Optional[QueryBudget] = ...,
    ) -> TxContext: ...

    def transact(
        self,
        ops: Optional[List[Any]] = None,
        *,
        dry_run: bool = False,
        max_cascade: Optional[int] = None,
        budget: Optional[QueryBudget] = None,
    ) -> Union[Report, TxContext]:
        """Submit a transaction.

        **Context-manager form** (no ``ops``): use as ``with db.transact() as tx:`` to
        build ops with :class:`TxBuilder` methods.  The report is on ``tx.report`` after
        the block.

        **Direct form** (with ``ops``): pass a list of bridge op dicts and receive a
        :class:`Report` directly.

        *budget* bounds the transaction; a stopped one commits nothing.
        """
        if ops is not None:
            args = _tx_args(ops, dry_run, max_cascade, budget)
            result_text = _call(self._native, "transact", json.dumps(args))
            return report_from_json(json.loads(result_text))
        return TxContext(
            self._native, dry_run=dry_run, max_cascade=max_cascade, budget=budget
        )

    def cypher_write(
        self,
        text: str,
        params: Optional[Dict[str, Any]] = None,
        *,
        dry_run: bool = False,
        budget: Optional[QueryBudget] = None,
    ) -> CypherWriteResult:
        """Run a Cypher statement that may write, in its own transaction.

        *budget* bounds the statement; a stopped one commits nothing.
        """
        args: Dict[str, Any] = {"text": text}
        if params:
            args["params"] = params
        if dry_run:
            args["options"] = {"dryRun": True}
        if budget is not None:
            args["budget"] = budget._to_json()
        result_text = _call(self._native, "cypherWrite", json.dumps(args))
        j = json.loads(result_text)
        rep_j = j.get("report")
        return CypherWriteResult(
            columns=list(j.get("columns") or []),
            rows=list(j.get("rows") or []),
            report=report_from_json(rep_j) if rep_j else None,
        )

    def speculate(
        self,
        ops: List[Any],
        queries: List[Dict[str, Any]],
    ) -> List[Any]:
        """Apply *ops* hypothetically, run *queries* on the result, keep nothing.

        Returns a list of query results (one per entry in *queries*).
        """
        args: Dict[str, Any] = {"ops": ops, "queries": queries}
        result_text = _call(self._native, "with", json.dumps(args))
        j: Dict[str, Any] = json.loads(result_text)
        return list(j.get("results") or [])

    def cancel(self, key: str) -> bool:
        """Cancel the call running with ``QueryBudget(cancel_key=key)``.

        A call that has not started yet is cancelled as soon as it starts, so use a
        fresh key per call. Returns whether a running call held the key. Safe to call
        from another thread: the GIL is released while a call runs.
        """
        result_text = _call(self._native, "cancel", json.dumps({"key": key}))
        return bool(json.loads(result_text)["running"])

    def bulk_import(self) -> BulkImport:
        """Start a bulk import session: chunked atomic writes, statistics refreshed once.

        Raises ``ImportInProgress`` while another session is open. Use it as a
        context manager (``with db.bulk_import() as imp:``) so the session always
        ends.
        """
        j = json.loads(_call(self._native, "importBegin", ""))
        return BulkImport(self._native, int(j["session"]))

    def rebuild_text_index(self) -> int:
        """Drop and rebuild the derived text index from every statement (history is
        untouched); returns the number of indexed string values. Also builds a missing
        index."""
        return int(json.loads(_call(self._native, "rebuildTextIndex", ""))["values"])

    def enable_text_index(self) -> bool:
        """Build the text index unless it is current; returns whether it built the
        whole index."""
        return bool(json.loads(_call(self._native, "enableTextIndex", ""))["built"])

    def optimize(self) -> None:
        """Refresh query-planner statistics after large imports."""
        _call(self._native, "optimize", "")

    def info(self) -> Dict[str, Any]:
        """Return database information: path, reader count, path limits, whether an
        import session is active (``importActive``) and whether planner statistics are
        due (``statisticsDue``)."""
        result_text = _call(self._native, "info", "")
        return dict(json.loads(result_text))
