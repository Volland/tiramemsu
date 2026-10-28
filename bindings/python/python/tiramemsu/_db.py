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
    ConstraintBudget,
    CypherResult,
    CypherWriteResult,
    ImportProgress,
    ImportSummary,
    PathCompleteness,
    PathReport,
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
    Invalidation,
    invalidation_from_json,
    SavedAnswer,
    saved_answer_from_json,
    sparql_result_from_json,
    statement_from_json,
    term_from_json,
    text_hit_from_json,
    TextHit,
    recall_hit_from_json,
    RecallHit,
    vector_result_from_json,
    VectorResult,
    BundlePreview,
    bundle_preview_from_json,
    Conflict,
    conflict_from_json,
    term_to_json,
    time_to_json,
)

def _model_json(model: Any) -> Any:
    """An embedding model: a plain ``str`` is its IRI, anything else is a term."""
    if isinstance(model, str):
        return model
    return term_to_json(model)


def _vector_json(vector: Any) -> List[float]:
    """A sequence of numbers as a JSON list."""
    return [float(x) for x in vector]


# A sentinel that is distinct from None (which means "clear the bound").
_UNSET: Any = object()


# ------------------------------------------------------------------- N3 reasoning helpers

_LIMIT_KEYS = {
    "max_iterations": "maxIterations",
    "max_match_steps": "maxMatchSteps",
    "max_backward_depth": "maxBackwardDepth",
    "max_term_bytes": "maxTermBytes",
}


def _rule_set_json(rule_set: Any) -> Any:
    """A rule set: a plain ``str`` is its IRI, anything else is a term."""
    if isinstance(rule_set, str):
        return rule_set
    return term_to_json(rule_set)


def _scope_args(
    args: Dict[str, Any],
    scope: Any,
    window: Any,
    max_segments: Optional[int],
    limits: Optional[Dict[str, int]],
) -> Dict[str, Any]:
    """The scope, window, segment and limit arguments of ``reason`` and ``materialize``."""
    if scope is not None:
        if scope == "all":
            args["scope"] = "all"
        elif "graphs" in scope:
            args["scope"] = {"graphs": [term_to_json(g) for g in scope["graphs"]]}
        elif "seeds" in scope:
            seeds = scope["seeds"]
            out: Dict[str, Any] = {"nodes": [term_to_json(n) for n in seeds["nodes"]]}
            if seeds.get("hops") is not None:
                out["hops"] = seeds["hops"]
            args["scope"] = {"seeds": out}
        else:
            raise TypeError('scope must be "all", {"graphs": [...]} or {"seeds": {...}}')
    if window is not None:
        start, end = window
        w: Dict[str, Any] = {}
        if start is not None:
            w["from"] = time_to_json(start)
        if end is not None:
            w["to"] = time_to_json(end)
        args["window"] = w
    if max_segments is not None:
        args["maxSegments"] = max_segments
    if limits:
        args["limits"] = {_LIMIT_KEYS.get(k, k): v for k, v in limits.items()}
    return args


def _conclusion_term(j: Any) -> Any:
    """A conclusion term: a stored term, or ``{"blank"}``, ``{"list"}``, ``{"formula"}``."""
    if isinstance(j, dict):
        if "list" in j:
            return {"list": [_conclusion_term(x) for x in j["list"]]}
        if "blank" in j or "formula" in j:
            return j
    return term_from_json(j)


def _conclusion(c: Dict[str, Any]) -> Dict[str, Any]:
    proof = dict(c["proof"])
    if proof.get("ruleSet") is not None:
        proof["ruleSet"] = term_from_json(proof["ruleSet"])
    return {
        **c,
        "s": _conclusion_term(c["s"]),
        "p": _conclusion_term(c["p"]),
        "o": _conclusion_term(c["o"]),
        "proof": proof,
    }


# ------------------------------------------------------------------------ Datalog helpers

_DATALOG_LIMIT_KEYS = {
    "max_iterations": "maxIterations",
    "max_facts": "maxFacts",
    "max_fragments": "maxFragments",
    "max_new_values": "maxNewValues",
}


def _datalog_args(
    args: Dict[str, Any],
    program: Optional[str],
    stored: Any,
    scope: Any = None,
    window: Any = None,
    time: Optional[str] = None,
    limits: Optional[Dict[str, int]] = None,
    demand: Optional[bool] = None,
    proof: bool = False,
    access: Optional[str] = None,
    parallel_threshold: Optional[int] = None,
    require_complete: bool = False,
) -> Dict[str, Any]:
    """The program and run options of the Datalog reads."""
    _scope_args(args, scope, window, None, None)
    if program is not None:
        args["program"] = program
    if stored is not None:
        args["stored"] = _rule_set_json(stored)
    if time is not None:
        args["time"] = time
    if limits:
        args["limits"] = {_DATALOG_LIMIT_KEYS.get(k, k): v for k, v in limits.items()}
    if demand is not None:
        args["demand"] = demand
    if proof:
        args["proof"] = True
    if access is not None:
        args["access"] = access
    if parallel_threshold is not None:
        args["parallelThreshold"] = parallel_threshold
    if require_complete:
        args["requireComplete"] = True
    return args


def _datalog_term(j: Any) -> Any:
    """A result term: a stored term, or ``{"invented": n}`` for a node a rule invented."""
    if isinstance(j, dict) and "invented" in j:
        return j
    return term_from_json(j)


def _fact(f: Dict[str, Any]) -> Dict[str, Any]:
    return {**f, "args": [_datalog_term(a) for a in f["args"]]}


def _program_node(r: Dict[str, Any], key: str) -> None:
    if r.get(key) is not None:
        r[key] = term_from_json(r[key])


def _datalog_result(r: Dict[str, Any]) -> Dict[str, Any]:
    r["rows"] = [{**row, "values": [_datalog_term(x) for x in row["values"]]} for row in r["rows"]]
    r["facts"] = [_fact(f) for f in r["facts"]]
    _program_node(r, "program")
    return r


def _path_row(r: Dict[str, Any]) -> PathRow:
    """Decode one row of the bridge's ``path`` operation."""
    return PathRow(
        start=term_from_json(r["start"]),
        end=term_from_json(r["end"]),
        hops=int(r["hops"]),
        path=r.get("path") if r.get("path") is not None else None,
        arrival=None if r.get("arrival") is None else int(r["arrival"]),
    )


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

    def derive(
        self,
        s: Any,
        p: Any,
        o: Any,
        support: List[Any],
        *,
        valid_from: Any = None,
        valid_to: Any = None,
    ) -> Ref:
        """Assert the belief ``(s, p, o)`` as derived from every statement in *support*.

        Writes the statement and one engine-owned ``sys:derivedFrom`` link per support.
        When any support is retracted or superseded, the belief is retracted with it.
        Deriving the same content from the same live supports again returns the existing
        statement.  Returns a :class:`Ref`.
        """
        name = self._fresh()
        op: Dict[str, Any] = {
            "op": "derive",
            "s": self._v(s),
            "p": self._v(p),
            "o": self._v(o),
            "support": [self._eid(e) for e in support],
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

    def new_node(self) -> Ref:
        """Allocate a fresh anonymous node.  Returns a :class:`Ref` usable as a
        subject or object in later operations of the same transaction."""
        name = self._fresh()
        self._ops.append({"op": "newNode", "as": name})
        return Ref(name)

    def define_embedding_model(
        self,
        model: Any,
        dimensions: int,
        *,
        metric: str = "cosine",
        encoding: str = "f32",
    ) -> None:
        """Define embedding model *model* (an IRI string or :class:`Iri`): its number
        of *dimensions*, *metric* (``"cosine"``, ``"dot"`` or ``"l2"``) and storage
        *encoding* (``"f32"`` or ``"f16"``).  Defining the identical model again is a
        no-op; a different definition for the same IRI raises ``InvalidQuery``."""
        self._ops.append(
            {
                "op": "defineEmbeddingModel",
                "model": _model_json(model),
                "dimensions": dimensions,
                "metric": metric,
                "encoding": encoding,
            }
        )

    def set_embedding(self, eid: Any, model: Any, vector: Any) -> None:
        """Store the embedding of live statement *eid* (int, :class:`Stmt` or a
        :class:`Ref` from this transaction) under *model*.  The store never computes
        embeddings: pass the vector your model produced.  The identical vector again
        is a no-op; a different one raises ``EmbeddingConflict``."""
        self._ops.append(
            {
                "op": "setEmbedding",
                "eid": self._eid(eid),
                "model": _model_json(model),
                "vector": _vector_json(vector),
            }
        )

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

    def store_rules(self, rule_set: Any, n3: str) -> Ref:
        """Store N3 rule text as the rule set *rule_set* (an IRI ``str`` or a term).

        Writes one statement ``(rule_set n3:source "<n3>")``. New text supersedes the
        live version (its materialized conclusions are retracted); equal text returns the
        live statement. Text that does not parse fails the transaction with ``N3Parse``.
        Returns a :class:`Ref` to the rule-set statement.
        """
        name = self._fresh()
        self._ops.append(
            {"op": "storeRules", "ruleSet": _rule_set_json(rule_set), "n3": n3, "as": name}
        )
        return Ref(name)

    def store_program(self, program: Any, text: str) -> Ref:
        """Store Datalog text as the program *program* (an IRI ``str`` or a term).

        Writes one statement ``(program dl:source "<text>")``. New text supersedes the
        live version; equal text returns the live statement. Text that does not compile
        fails the transaction (``DatalogParse``, ``DatalogUnsafe``, ...), and so does a
        node that already holds N3 rules (``RuleLanguageConflict``). Returns a
        :class:`Ref` to the program statement.
        """
        name = self._fresh()
        self._ops.append(
            {"op": "storeProgram", "program": _rule_set_json(program), "text": text, "as": name}
        )
        return Ref(name)

    def enforce(self, program: Any, on: bool = True) -> Optional[Ref]:
        """Enforce the stored program *program* (an IRI ``str`` or a term) at every
        commit, or stop enforcing it with ``on=False``.

        Writes one statement ``(program dl:enforced true)``. Turning it on checks the
        program's ``error`` constraints over the whole store first and fails with
        ``ConstraintViolated`` when the data already violates one, or
        ``ConstraintNotEnforceable`` when one cannot be checked from a transaction's
        changes. From then on a commit that would introduce a violation is refused with
        ``ConstraintViolated`` (its violations are on the error's ``violations``).
        Returns a :class:`Ref` to the ``dl:enforced`` statement, or ``None`` when
        turning enforcement off.
        """
        op: Dict[str, Any] = {"op": "enforce", "program": _rule_set_json(program), "on": on}
        if not on:
            self._ops.append(op)
            return None
        name = self._fresh()
        op["as"] = name
        self._ops.append(op)
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
        constraint_budget: Optional[ConstraintBudget] = None,
    ) -> None:
        self._native = native
        self._dry_run = dry_run
        self._max_cascade = max_cascade
        self._budget = budget
        self._constraint_budget = constraint_budget
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
            self._builder._ops,
            self._dry_run,
            self._max_cascade,
            self._budget,
            self._constraint_budget,
        )
        args.update(self._extra)
        result_text = _call(self._native, self._op, json.dumps(args))
        self._builder.report = report_from_json(json.loads(result_text))


def _tx_args(
    ops: List[Any],
    dry_run: bool,
    max_cascade: Optional[int],
    budget: Optional[QueryBudget],
    constraint_budget: Optional[ConstraintBudget] = None,
) -> Dict[str, Any]:
    """The bridge arguments of a transaction (or import chunk) of *ops*."""
    args: Dict[str, Any] = {"ops": ops}
    options: Dict[str, Any] = {}
    if dry_run:
        options["dryRun"] = True
    if max_cascade is not None:
        options["maxCascade"] = max_cascade
    if constraint_budget is not None:
        options["constraintBudget"] = constraint_budget._to_json()
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
        constraint_budget: Optional[ConstraintBudget] = ...,
    ) -> Report: ...

    @overload
    def chunk(
        self,
        ops: None = None,
        *,
        dry_run: bool = ...,
        max_cascade: Optional[int] = ...,
        budget: Optional[QueryBudget] = ...,
        constraint_budget: Optional[ConstraintBudget] = ...,
    ) -> TxContext: ...

    def chunk(
        self,
        ops: Optional[List[Any]] = None,
        *,
        dry_run: bool = False,
        max_cascade: Optional[int] = None,
        budget: Optional[QueryBudget] = None,
        constraint_budget: Optional[ConstraintBudget] = None,
    ) -> Union[Report, TxContext]:
        """Commit one chunk, in the two forms of :meth:`Database.transact`.

        A failing chunk raises and rolls back alone; earlier chunks stay committed.
        """
        extra = {"session": self.session}
        if ops is not None:
            args = _tx_args(ops, dry_run, max_cascade, budget, constraint_budget)
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
            constraint_budget=constraint_budget,
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

    def sparql(
        self,
        text: str,
        *,
        provenance: bool = False,
        query_only: bool = False,
        params: Optional[Dict[str, Any]] = None,
        path_completeness: bool = False,
    ) -> SparqlResult:
        """Run a SPARQL query on this view.

        Returns :class:`~._types.SparqlSelectResult`,
        :class:`~._types.SparqlAskResult`, :class:`~._types.SparqlGraphResult`, or
        :class:`~._types.SparqlUpdateResult` depending on the query form.  With
        *provenance*, a SELECT result also lists the statements behind each row;
        with *query_only*, an update raises ``Unsupported`` before anything runs.
        *params* gives the start instants (epoch ms, ``datetime`` or ISO text) of
        ``SERVICE <urn:tiramemsu:tm:timeRespecting/$name>`` scopes; with
        *path_completeness*, a SELECT result reports how completely its paths ran.
        """
        args: Dict[str, Any] = {"text": text}
        if provenance:
            args["provenance"] = True
        if query_only:
            args["queryOnly"] = True
        if params:
            args["params"] = {k: time_to_json(v) for k, v in params.items()}
        if path_completeness:
            args["pathCompleteness"] = True
        result_text = self._call("sparql", args)
        return sparql_result_from_json(json.loads(result_text))

    def cypher(
        self,
        text: str,
        params: Optional[Dict[str, Any]] = None,
        *,
        path_completeness: bool = False,
    ) -> CypherResult:
        """Run a read-only Cypher query on this view.  With *path_completeness*, the
        result reports how completely its variable-length patterns were evaluated
        (``"cap"`` when ``path_max_hops`` cut one)."""
        args: Dict[str, Any] = {"text": text}
        if params:
            args["params"] = params
        if path_completeness:
            args["pathCompleteness"] = True
        result_text = self._call("cypher", args)
        j = json.loads(result_text)
        return CypherResult(
            columns=list(j.get("columns") or []),
            rows=list(j.get("rows") or []),
            path_completeness=PathCompleteness.from_json(j.get("pathCompleteness")),
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
        capped: bool = False,
    ) -> List[PathRow]:
        """Find all nodes reachable from *start* via *path_expr*.

        *mode* is ``"reach"`` (default), ``"trail"``, ``"anyShortest"`` or
        ``"allShortest"``.  *graphs* keeps every hop inside the listed graphs (a term
        that is not stored names no graph).  *time_respecting* is ``True`` or a time
        to start after: each hop must start no earlier than the previous one, and
        every row carries its ``arrival``.  With *capped* and no *max_hops*, the
        search stops at the database's ``path_max_hops`` (as a Cypher ``*`` does).
        """
        args = self._path_args(start, path_expr, mode, max_hops, graphs, time_respecting, capped)
        rows: List[Any] = json.loads(self._call("path", args))
        return [_path_row(r) for r in rows]

    def path_report(
        self,
        start: Any,
        path_expr: str,
        *,
        mode: str = "reach",
        max_hops: Optional[int] = None,
        graphs: Optional[List[Any]] = None,
        time_respecting: Any = False,
        capped: bool = False,
    ) -> PathReport:
        """:meth:`path` with how completely the search was evaluated: a
        :class:`~._types.PathReport` whose ``completeness.kind`` is
        ``"exhaustive"``, ``"bound"`` or ``"cap"``."""
        args = self._path_args(start, path_expr, mode, max_hops, graphs, time_respecting, capped)
        args["completeness"] = True
        j = json.loads(self._call("path", args))
        completeness = PathCompleteness.from_json(j["completeness"])
        assert completeness is not None
        return PathReport([_path_row(r) for r in j["rows"]], completeness)

    @staticmethod
    def _path_args(
        start: Any,
        path_expr: str,
        mode: str,
        max_hops: Optional[int],
        graphs: Optional[List[Any]],
        time_respecting: Any,
        capped: bool,
    ) -> Dict[str, Any]:
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
        if capped:
            args["capped"] = True
        return args

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

    def vector_search(
        self,
        model: Any,
        vector: Any,
        *,
        limit: Optional[int] = None,
        graphs: Optional[List[Any]] = None,
        predicates: Optional[List[Any]] = None,
        prefilter: Optional[int] = None,
        confidence: Any = None,
    ) -> VectorResult:
        """Vector recall: the statements of this view whose embedding under *model*
        is closest to *vector*, ranked by similarity, then confidence,
        confirmations, authors and recency, then eid (``tiramemsu-vector-rank/1``).

        The search is exact over the statements this view selects, so as-of,
        history and valid-time views never lose a hit.  *limit* defaults to 10.
        *prefilter* scores only that many candidates chosen by sign-bit distance and
        re-ranks them exactly (``result.exact`` is then false).  Works on every
        SQLite build; no extension is needed.
        """
        args: Dict[str, Any] = {"model": _model_json(model), "vector": _vector_json(vector)}
        if limit is not None:
            args["limit"] = limit
        if graphs is not None:
            args["graphs"] = [term_to_json(g) for g in graphs]
        if predicates is not None:
            args["predicates"] = [term_to_json(p) for p in predicates]
        if prefilter is not None:
            args["prefilter"] = prefilter
        if confidence is not None:
            args["confidence"] = term_to_json(confidence)
        return vector_result_from_json(json.loads(self._call("vectorSearch", args)))

    def recall(
        self,
        text: Optional[str] = None,
        *,
        model: Any = None,
        vector: Any = None,
        mode: str = "all",
        limit: Optional[int] = None,
        depth: Optional[int] = None,
        expand: int = 0,
        graphs: Optional[List[Any]] = None,
        predicates: Optional[List[Any]] = None,
    ) -> List[RecallHit]:
        """Hybrid recall: text recall of *text* and vector recall of *vector* under
        *model* in this view, fused by reciprocal rank (``tiramemsu-recall-rank/1``).
        Give text, a model with a vector, or both.  *expand* adds, per fused hit, up
        to that many of its layers and facts about its subject (``via`` set)."""
        args: Dict[str, Any] = {"mode": mode, "expand": expand}
        if text is not None:
            args["text"] = text
        if model is not None:
            args["model"] = _model_json(model)
        if vector is not None:
            args["vector"] = _vector_json(vector)
        if limit is not None:
            args["limit"] = limit
        if depth is not None:
            args["depth"] = depth
        if graphs is not None:
            args["graphs"] = [term_to_json(g) for g in graphs]
        if predicates is not None:
            args["predicates"] = [term_to_json(p) for p in predicates]
        rows: List[Any] = json.loads(self._call("recall", args))
        return [recall_hit_from_json(r) for r in rows]

    def unembedded(self, model: Any, limit: Optional[int] = None) -> List[int]:
        """The eids of statements in this view with no embedding under *model*, in
        eid order: the backlog of an embedding job."""
        args: Dict[str, Any] = {"model": _model_json(model)}
        if limit is not None:
            args["limit"] = limit
        return [int(e) for e in json.loads(self._call("unembedded", args))]

    def conflicts(
        self,
        *,
        s: Any = None,
        p: Any = None,
        limit: Optional[int] = None,
        confidence: Any = None,
        source: Any = None,
    ) -> List["Conflict"]:
        """Subject/predicate pairs of this view with distinct objects whose valid
        intervals overlap, with the attributed evidence of every value.

        Statements with the same object support one value; disjoint valid times are
        not reported. Nothing is scored, resolved or written. *s* and *p* filter,
        *limit* caps the count, and *confidence* / *source* name the layer
        predicates (default ``v:confidence`` and ``v:source``). Raises
        ``Unsupported`` on the history view.
        """
        args: Dict[str, Any] = {}
        for key, val in (("s", s), ("p", p), ("confidence", confidence), ("source", source)):
            if val is not None:
                args[key] = term_to_json(val)
        if limit is not None:
            args["limit"] = limit
        rows: List[Any] = json.loads(self._call("conflicts", args))
        return [conflict_from_json(r) for r in rows]

    def explain_sparql(self, text: str) -> Dict[str, Any]:
        """Explain SPARQL query text without running it.

        Returns ``{"regions": [{"kind", "note", "aliases", "queryPlan"}], "sql",
        "shortCircuit", "queryPlan"}``: where each region runs (``"sql"``,
        ``"nativePath"``, ``"nativeLftj"`` or ``"native"`` on redb) and why (``"lftjNative"``,
        ``"cyclicLftjDisabled"``, ``"lftjBelowEstimate"``, ...), the generated SQL and
        SQLite's ``EXPLAIN QUERY PLAN``.  Raises ``Unsupported`` for an update.
        """
        return dict(json.loads(self._call("explainSparql", {"text": text})))

    def values(self, s: Any, key: Any) -> List[Any]:
        """Return all objects ``o`` where ``(s, key, o)`` exists in this view."""
        items: List[Any] = json.loads(
            self._call("values", {"s": term_to_json(s), "key": term_to_json(key)})
        )
        return [term_from_json(v) for v in items]

    def reason(
        self,
        rules: Optional[str] = None,
        *,
        rule_set: Any = None,
        scope: Any = None,
        window: Any = None,
        time: Optional[str] = None,
        max_segments: Optional[int] = None,
        proof_n3: bool = False,
        limits: Optional[Dict[str, int]] = None,
    ) -> Dict[str, Any]:
        """Run N3 rules over this view and return what follows, with proofs; writes nothing.

        Give the rules as N3 text (*rules*) or name a rule set stored with
        :meth:`TxBuilder.store_rules` (*rule_set*). A conclusion holds at an instant only if
        it follows from the statements valid then. *scope* is ``"all"``,
        ``{"graphs": [...]}`` or ``{"seeds": {"nodes": [...], "hops": n}}``; *window* is a
        ``(from, to)`` pair (either may be ``None``); *time* is ``"pointwise"`` (default) or
        ``"atemporal"``. Returns ``{"conclusions", "t", "now", "loaded", "pruned",
        "segments", "status", "limits", "errors", "contradiction", "ruleSet",
        "ruleSetStatement", "proofN3"}``; each conclusion is ``{"s", "p", "o",
        "validFrom", "validTo", "proof": {"ruleSet", "rule", "premises"}}``.
        """
        args = _scope_args({}, scope, window, max_segments, limits)
        if rules is not None:
            args["rules"] = rules
        if rule_set is not None:
            args["ruleSet"] = _rule_set_json(rule_set)
        if time is not None:
            args["time"] = time
        if proof_n3:
            args["proofN3"] = True
        r: Dict[str, Any] = json.loads(self._call("reason", args))
        r["conclusions"] = [_conclusion(c) for c in r["conclusions"]]
        if r.get("ruleSet") is not None:
            r["ruleSet"] = term_from_json(r["ruleSet"])
        return r

    def rule_set_info(self, rule_set: Any) -> Dict[str, Any]:
        """A stored rule set as this view sees it: ``{"ruleSet", "statement", "text",
        "rules", "monotonicity", "predicates", "variablePredicate"}``."""
        r: Dict[str, Any] = json.loads(
            self._call("ruleSetInfo", {"ruleSet": _rule_set_json(rule_set)})
        )
        r["ruleSet"] = term_from_json(r["ruleSet"])
        return r

    def datalog(
        self,
        goal: Optional[str] = None,
        program: Optional[str] = None,
        *,
        stored: Any = None,
        scope: Any = None,
        window: Any = None,
        time: Optional[str] = None,
        limits: Optional[Dict[str, int]] = None,
        demand: Optional[bool] = None,
        proof: bool = False,
        access: Optional[str] = None,
        parallel_threshold: Optional[int] = None,
        require_complete: bool = False,
    ) -> Dict[str, Any]:
        """Run a Datalog program over this view and answer *goal*; writes nothing.

        Give the program as Datalog text (*program*) or name a program stored with
        :meth:`TxBuilder.store_program` (*stored*); *goal* is a body such as
        ``"reports(v:alice, X)"`` (default: the program's ``?-`` goal). *scope*, *window*
        and *time* are those of :meth:`reason`; *limits* takes ``max_iterations``,
        ``max_facts``, ``max_fragments`` and ``max_new_values``; *demand* (default on)
        rewrites for the goal's bound arguments; *access* is ``"auto"``, ``"probe"`` or
        ``"load"``; *require_complete* raises ``DatalogIncomplete`` instead of returning a
        partial result. Returns ``{"columns", "rows", "facts", "t", "now", "strata",
        "derived", "predicates", "pruned", "demand", "status", "limits", "program",
        "programStatement"}``; each row is ``{"values", "valid": [{"from", "to"}]}``, with
        ``"proof"`` when *proof* is set. A node a rule invented is ``{"invented": n}``.
        """
        args = _datalog_args(
            {}, program, stored, scope, window, time, limits, demand, proof, access,
            parallel_threshold, require_complete,
        )
        if goal is not None:
            args["goal"] = goal
        return _datalog_result(json.loads(self._call("datalog", args)))

    def datalog_reason(
        self,
        program: Optional[str] = None,
        *,
        stored: Any = None,
        scope: Any = None,
        window: Any = None,
        time: Optional[str] = None,
        limits: Optional[Dict[str, int]] = None,
        demand: Optional[bool] = None,
        proof: bool = False,
        access: Optional[str] = None,
        parallel_threshold: Optional[int] = None,
        require_complete: bool = False,
        include_relations: bool = False,
    ) -> Dict[str, Any]:
        """Every triple a Datalog program concludes that is not among the stored
        statements; writes nothing.

        Takes the options of :meth:`datalog`; with *include_relations*, the facts of
        plain-named relations are returned too. Returns ``{"conclusions", "relations",
        "facts", "t", "now", "predicates", "status", "limits", "program",
        "programStatement"}``; each conclusion is ``{"s", "p", "o", "validFrom",
        "validTo", "proof"}``.
        """
        args = _datalog_args(
            {}, program, stored, scope, window, time, limits, demand, proof, access,
            parallel_threshold, require_complete,
        )
        if include_relations:
            args["includeRelations"] = True
        r: Dict[str, Any] = json.loads(self._call("datalogReason", args))
        r["conclusions"] = [
            {
                **c,
                "s": _datalog_term(c["s"]),
                "p": term_from_json(c["p"]),
                "o": _datalog_term(c["o"]),
            }
            for c in r["conclusions"]
        ]
        r["relations"] = [_fact(f) for f in r["relations"]]
        r["facts"] = [_fact(f) for f in r["facts"]]
        _program_node(r, "program")
        return r

    def program_info(self, program: Optional[str] = None, *, stored: Any = None) -> Dict[str, Any]:
        """What a program is as this view sees it: ``{"program", "statement", "text",
        "rules", "goals", "constraints", "strata", "predicates", "variablePredicate",
        "metricRecursion", "inventionRecursion"}``."""
        r: Dict[str, Any] = json.loads(
            self._call("programInfo", _datalog_args({}, program, stored))
        )
        _program_node(r, "program")
        return r

    def explain_datalog(
        self,
        goal: Optional[str] = None,
        program: Optional[str] = None,
        *,
        stored: Any = None,
        scope: Any = None,
        window: Any = None,
        time: Optional[str] = None,
        limits: Optional[Dict[str, int]] = None,
        demand: Optional[bool] = None,
        proof: bool = False,
        access: Optional[str] = None,
        parallel_threshold: Optional[int] = None,
        require_complete: bool = False,
    ) -> Dict[str, Any]:
        """Run a Datalog goal like :meth:`datalog` (same options) and return
        ``{"plan", "result"}``: the plan per stratum and rule, with access choices."""
        args = _datalog_args(
            {}, program, stored, scope, window, time, limits, demand, proof, access,
            parallel_threshold, require_complete,
        )
        if goal is not None:
            args["goal"] = goal
        r: Dict[str, Any] = json.loads(self._call("explainDatalog", args))
        r["result"] = _datalog_result(r["result"])
        return r

    def check(
        self,
        program: Optional[str] = None,
        *,
        only: Optional[List[str]] = None,
        stored: Any = None,
        scope: Any = None,
        window: Any = None,
        time: Optional[str] = None,
        limits: Optional[Dict[str, int]] = None,
        demand: Optional[bool] = None,
        proof: bool = False,
        access: Optional[str] = None,
        parallel_threshold: Optional[int] = None,
        require_complete: bool = False,
    ) -> Dict[str, Any]:
        """Check a program's constraints (or those named in *only*) over this view,
        pointwise in valid time; writes nothing.

        Takes the options of :meth:`datalog`. Returns ``{"violated", "checked", "t",
        "now", "status", "limits"}``; each violated constraint is ``{"name",
        "severity", "violations"}`` and each violation ``{"bindings", "eids",
        "valid"}``.
        """
        args = _datalog_args(
            {}, program, stored, scope, window, time, limits, demand, proof, access,
            parallel_threshold, require_complete,
        )
        if only is not None:
            args["only"] = list(only)
        r: Dict[str, Any] = json.loads(self._call("check", args))
        for c in r["violated"]:
            for v in c["violations"]:
                v["bindings"] = {k: _datalog_term(t) for k, t in v["bindings"].items()}
        return r


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
        lftj: Optional[bool] = None,
        lftj_min_rows: Optional[int] = None,
        backend: Optional[str] = None,
        durability: Optional[str] = None,
        flush_interval_ms: Optional[int] = None,
        flush_commits: Optional[int] = None,
        create: bool = False,
    ) -> None:
        options: Dict[str, Any] = {}
        if backend is not None:
            options["backend"] = backend
        if durability is not None:
            options["durability"] = durability
        if flush_interval_ms is not None:
            options["flushIntervalMs"] = flush_interval_ms
        if flush_commits is not None:
            options["flushCommits"] = flush_commits
        if create:
            options["create"] = True
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
        if lftj is not None:
            options["lftj"] = bool(lftj)
        if lftj_min_rows is not None:
            options["lftjMinRows"] = lftj_min_rows
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
        constraint_budget: Optional[ConstraintBudget] = ...,
    ) -> Report: ...

    @overload
    def transact(
        self,
        ops: None = None,
        *,
        dry_run: bool = ...,
        max_cascade: Optional[int] = ...,
        budget: Optional[QueryBudget] = ...,
        constraint_budget: Optional[ConstraintBudget] = ...,
    ) -> TxContext: ...

    def transact(
        self,
        ops: Optional[List[Any]] = None,
        *,
        dry_run: bool = False,
        max_cascade: Optional[int] = None,
        budget: Optional[QueryBudget] = None,
        constraint_budget: Optional[ConstraintBudget] = None,
    ) -> Union[Report, TxContext]:
        """Submit a transaction.

        **Context-manager form** (no ``ops``): use as ``with db.transact() as tx:`` to
        build ops with :class:`TxBuilder` methods.  The report is on ``tx.report`` after
        the block.

        **Direct form** (with ``ops``): pass a list of bridge op dicts and receive a
        :class:`Report` directly.

        *budget* bounds the transaction; a stopped one commits nothing.
        *constraint_budget* bounds the commit-time checks of enforced Datalog programs.
        """
        if ops is not None:
            args = _tx_args(ops, dry_run, max_cascade, budget, constraint_budget)
            result_text = _call(self._native, "transact", json.dumps(args))
            return report_from_json(json.loads(result_text))
        return TxContext(
            self._native,
            dry_run=dry_run,
            max_cascade=max_cascade,
            budget=budget,
            constraint_budget=constraint_budget,
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

    def save_answer(
        self,
        name: str,
        text: str,
        *,
        language: str = "sparql",
        params: Optional[Dict[str, Any]] = None,
        view: Optional["View"] = None,
        budget: Optional[QueryBudget] = None,
    ) -> SavedAnswer:
        """Run a query and save its answer under *name* (replacing one of that name).

        Stores the text, *params* and *view* (default: :meth:`now`) exactly as given,
        the current ``@vocab`` and prefixes, the result, the statements it cited
        (SPARQL SELECT provenance) and its ``coverage`` reasons. Writing it takes no
        transaction number and logs no event.
        """
        args: Dict[str, Any] = {"name": name, "text": text, "language": language}
        if params:
            args["params"] = params
        if view is not None:
            args["view"] = view._view
        if budget is not None:
            args["budget"] = budget._to_json()
        text_out = _call(self._native, "saveAnswer", json.dumps(args))
        return saved_answer_from_json(json.loads(text_out))

    def saved_answer(self, name: str) -> Optional[SavedAnswer]:
        """The saved answer *name* as stored, or ``None``. Call
        :meth:`check_saved_answers` first for current marks."""
        j = json.loads(_call(self._native, "savedAnswer", json.dumps({"name": name})))
        return None if j is None else saved_answer_from_json(j)

    def saved_answers(self) -> List[SavedAnswer]:
        """Every saved answer, by name."""
        items: List[Any] = json.loads(_call(self._native, "savedAnswers", ""))
        return [saved_answer_from_json(a) for a in items]

    def check_saved_answers(self) -> List[Invalidation]:
        """Process the events since each answer's cursor and return the new
        invalidations: a retracted cited statement makes an answer ``"stale"``; any
        other later transaction under a mutable view makes it ``"recheck"``.
        Idempotent: nothing is reported twice."""
        items: List[Any] = json.loads(_call(self._native, "checkSavedAnswers", ""))
        return [invalidation_from_json(i) for i in items]

    def refresh_answer(self, name: str, *, budget: Optional[QueryBudget] = None) -> SavedAnswer:
        """Re-run a saved answer with its stored query, view and settings. Only a
        success clears its mark and advances its checkpoint; a failure raises and
        leaves it as it was (``error`` records why)."""
        args: Dict[str, Any] = {"name": name}
        if budget is not None:
            args["budget"] = budget._to_json()
        text_out = _call(self._native, "refreshAnswer", json.dumps(args))
        return saved_answer_from_json(json.loads(text_out))

    def delete_saved_answer(self, name: str) -> bool:
        """Delete a saved answer; returns whether there was one."""
        j = json.loads(_call(self._native, "deleteSavedAnswer", json.dumps({"name": name})))
        return bool(j["deleted"])

    def preview_bundle(
        self, bundle: Dict[str, Any], *, budget: Optional[QueryBudget] = None
    ) -> BundlePreview:
        """Preview importing *bundle* now: a dry run with the full validation and
        write semantics that commits nothing (only burned id counters advance).

        A schema failure is reported in ``failure``, not raised; a malformed bundle
        raises ``InvalidTerm``. Apply it later with :meth:`TxBuilder.import_bundle`,
        which validates again against the database as it is then.
        """
        args: Dict[str, Any] = {"bundle": bundle}
        if budget is not None:
            args["budget"] = budget._to_json()
        return bundle_preview_from_json(
            json.loads(_call(self._native, "previewBundle", json.dumps(args)))
        )

    def reason(self, rules: Optional[str] = None, **kwargs: Any) -> Dict[str, Any]:
        """:meth:`View.reason` on the live database (:meth:`now`)."""
        return self.now().reason(rules, **kwargs)

    def rule_set_info(self, rule_set: Any) -> Dict[str, Any]:
        """:meth:`View.rule_set_info` on the live database (:meth:`now`)."""
        return self.now().rule_set_info(rule_set)

    def datalog(
        self, goal: Optional[str] = None, program: Optional[str] = None, **kwargs: Any
    ) -> Dict[str, Any]:
        """:meth:`View.datalog` on the live database (:meth:`now`)."""
        return self.now().datalog(goal, program, **kwargs)

    def datalog_reason(self, program: Optional[str] = None, **kwargs: Any) -> Dict[str, Any]:
        """:meth:`View.datalog_reason` on the live database (:meth:`now`)."""
        return self.now().datalog_reason(program, **kwargs)

    def program_info(self, program: Optional[str] = None, *, stored: Any = None) -> Dict[str, Any]:
        """:meth:`View.program_info` on the live database (:meth:`now`)."""
        return self.now().program_info(program, stored=stored)

    def explain_datalog(
        self, goal: Optional[str] = None, program: Optional[str] = None, **kwargs: Any
    ) -> Dict[str, Any]:
        """:meth:`View.explain_datalog` on the live database (:meth:`now`)."""
        return self.now().explain_datalog(goal, program, **kwargs)

    def check(self, program: Optional[str] = None, **kwargs: Any) -> Dict[str, Any]:
        """:meth:`View.check` on the live database (:meth:`now`)."""
        return self.now().check(program, **kwargs)

    def materialize(
        self,
        rule_set: Any,
        *,
        scope: Any = None,
        window: Any = None,
        max_segments: Optional[int] = None,
        limits: Optional[Dict[str, int]] = None,
        require_complete: bool = False,
        dry_run: bool = False,
        budget: Optional[QueryBudget] = None,
    ) -> Dict[str, Any]:
        """Reason with the stored rule set *rule_set* and write its conclusions as derived
        beliefs in one transaction, supported by their premises and the rule-set statement.

        Non-monotonic and clock-reading rule sets raise ``N3NotMaterializable``; a fired
        inference fuse raises ``N3Contradiction``. With *dry_run*, reports what would be
        written and writes nothing. Returns ``{"ruleSet", "rulesStatement",
        "reasonedAt", "report", "derived", "existing", "widened", "skolems",
        "staleSupport", "unstorable", "status", "limits", "dryRun"}``.
        """
        args = _scope_args(
            {"ruleSet": _rule_set_json(rule_set)}, scope, window, max_segments, limits
        )
        if require_complete:
            args["requireComplete"] = True
        if dry_run:
            args["dryRun"] = True
        if budget is not None:
            args["budget"] = budget._to_json()
        r: Dict[str, Any] = json.loads(_call(self._native, "materialize", json.dumps(args)))
        r["ruleSet"] = term_from_json(r["ruleSet"])
        r["skolems"] = [term_from_json(s) for s in r["skolems"]]
        r["staleSupport"] = [_conclusion(c) for c in r["staleSupport"]]
        r["unstorable"] = [_conclusion(c) for c in r["unstorable"]]
        return r

    def optimize(self) -> None:
        """Refresh query-planner statistics after large imports."""
        _call(self._native, "optimize", "")

    def sync(self) -> None:
        """Make every committed transaction durable (a durable commit on redb, a
        full WAL checkpoint on SQLite)."""
        _call(self._native, "sync", "")

    def compact(self) -> bool:
        """Reclaim free space in a redb file; returns False on SQLite."""
        return bool(json.loads(_call(self._native, "compact", ""))["compacted"])

    @property
    def backend(self) -> str:
        """The storage backend of this database: ``"sqlite"`` or ``"redb"``."""
        return str(self.info()["backend"])

    def info(self) -> Dict[str, Any]:
        """Return database information: path, reader count, path limits, whether an
        import session is active (``importActive``) and whether planner statistics are
        due (``statisticsDue``)."""
        result_text = _call(self._native, "info", "")
        return dict(json.loads(result_text))
