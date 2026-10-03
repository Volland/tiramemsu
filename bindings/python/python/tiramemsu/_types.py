"""Term types, result dataclasses and JSON conversion helpers for tiramemsu.

Every value that crosses the bridge is one of the Python scalars (``str``, ``int``,
``float``, ``bool``) or one of the frozen dataclasses below.  Conversion functions
:func:`term_to_json` and :func:`term_from_json` translate between Python values and
the bridge's JSON forms.
"""

from __future__ import annotations

import json as _json
import re as _re
from dataclasses import dataclass, field
from datetime import date, datetime, timedelta, timezone
from typing import Any, Dict, Iterator, List, Optional, Union


# ------------------------------------------------------------------------------- errors


class TiramemsuError(Exception):
    """A tiramemsu database error.  ``code`` is a machine-readable error kind string."""

    def __init__(self, code: str, message: str) -> None:
        super().__init__(message)
        self.code = code

    @classmethod
    def _from_json(cls, text: str) -> "TiramemsuError":
        try:
            d = _json.loads(text)
            code = str(d.get("code", "Error"))
            msg = str(d.get("message", text))
        except Exception:
            code = "Error"
            msg = text
        return cls(code, msg)

    def __str__(self) -> str:
        return f"[{self.code}] {super().__str__()}"


# ------------------------------------------------------------------------------- terms


@dataclass(frozen=True)
class QueryBudget:
    """The bounds of one call, for :meth:`View.with_budget` and ``budget=`` on writes.

    Every field is optional and independent; ``QueryBudget()`` bounds nothing. A call
    that runs past ``timeout_ms`` fails with code ``DeadlineExceeded``, one waiting
    longer than ``reader_timeout_ms`` for a reader with ``PoolTimeout``, one decoding
    more than ``max_rows`` rows or ``max_bytes`` bytes (across every statement it runs)
    with ``ResultLimitExceeded``, and one stopped by :meth:`Database.cancel` with
    ``Cancelled``. A stopped write commits nothing.
    """

    timeout_ms: Optional[int] = None
    reader_timeout_ms: Optional[int] = None
    max_rows: Optional[int] = None
    max_bytes: Optional[int] = None
    cancel_key: Optional[str] = None

    def _to_json(self) -> Dict[str, Any]:
        out: Dict[str, Any] = {}
        if self.timeout_ms is not None:
            out["timeoutMs"] = self.timeout_ms
        if self.reader_timeout_ms is not None:
            out["readerTimeoutMs"] = self.reader_timeout_ms
        if self.max_rows is not None:
            out["maxRows"] = self.max_rows
        if self.max_bytes is not None:
            out["maxBytes"] = self.max_bytes
        if self.cancel_key is not None:
            out["cancelKey"] = self.cancel_key
        return out


@dataclass(frozen=True)
class Iri:
    """An IRI (named node) term."""

    value: str

    def __str__(self) -> str:
        return f"<{self.value}>"

    def _to_json(self) -> Any:
        return {"iri": self.value}


@dataclass(frozen=True)
class Literal:
    """An RDF literal with an optional datatype IRI or language tag."""

    lex: str
    datatype: Optional[str] = None
    lang: Optional[str] = None

    def _to_json(self) -> Any:
        if self.lang:
            return {"lex": self.lex, "lang": self.lang}
        if self.datatype:
            return {"lex": self.lex, "datatype": self.datatype}
        return self.lex


@dataclass(frozen=True)
class Node:
    """An anonymous node identified by its integer id."""

    n: int

    def _to_json(self) -> Any:
        return {"node": self.n}


@dataclass(frozen=True)
class BNode:
    """A blank node identified by its integer id."""

    n: int

    def _to_json(self) -> Any:
        return {"bnode": self.n}


@dataclass(frozen=True)
class Stmt:
    """A statement id (eid)."""

    eid: int

    def _to_json(self) -> Any:
        return {"stmt": self.eid}


@dataclass(frozen=True)
class TxId:
    """A transaction id."""

    n: int

    def _to_json(self) -> Any:
        return {"tx": self.n}


# A Term is any tiramemsu value that Python can produce or consume.
Term = Union[str, int, float, bool, Iri, Literal, Node, BNode, Stmt, TxId, datetime, date]

_XSD = "http://www.w3.org/2001/XMLSchema#"


def term_to_json(v: Any) -> Any:
    """Convert a Python value to its bridge JSON form.

    Accepts :data:`Term` values plus :class:`Ref` (a transaction handle) for ops
    that use the ``as``/``ref`` mechanism.  Raises :exc:`TypeError` for unknown types.
    """
    # Ref is defined in _db to avoid a circular import; it adds its own _to_json.
    if hasattr(v, "_to_json"):
        return v._to_json()
    # bool must come before int because bool is a subclass of int.
    if isinstance(v, bool):
        return v
    if isinstance(v, int):
        if -9_007_199_254_740_992 <= v <= 9_007_199_254_740_992:
            return v
        return {"$int": str(v)}
    if isinstance(v, float):
        return v
    if isinstance(v, str):
        return v
    if isinstance(v, datetime):
        # A naive datetime is taken as UTC.
        utc = v.replace(tzinfo=timezone.utc) if v.tzinfo is None else v.astimezone(timezone.utc)
        lex = utc.replace(tzinfo=None).isoformat(timespec="milliseconds") + "Z"
        return {"lex": lex, "datatype": _XSD + "dateTime"}
    if isinstance(v, date):
        return {"lex": v.isoformat(), "datatype": _XSD + "date"}
    raise TypeError(f"cannot convert {type(v).__name__!r} to a term")


_DATETIME = _re.compile(
    r"^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2}):(\d{2})(?:\.(\d+))?(Z|[+-]\d{2}:\d{2})?$"
)


def _parse_datetime(lex: str) -> Optional[datetime]:
    """Parse an ``xsd:dateTime`` lexical form; a value with no offset is taken as UTC."""
    m = _DATETIME.match(lex)
    if m is None:
        return None
    y, mo, d, h, mi, sec, frac, tz = m.groups()
    micro = int((frac or "0").ljust(6, "0")[:6])
    if tz is None or tz == "Z":
        zone = timezone.utc
    else:
        sign = 1 if tz[0] == "+" else -1
        zone = timezone(sign * timedelta(hours=int(tz[1:3]), minutes=int(tz[4:6])))
    try:
        return datetime(int(y), int(mo), int(d), int(h), int(mi), int(sec), micro, tzinfo=zone)
    except ValueError:
        return None


def term_from_json(j: Any) -> Any:
    """Convert a bridge JSON value to a Python term.

    Returns the most specific Python type: :class:`Iri`, :class:`Node`, :class:`BNode`,
    :class:`Stmt`, :class:`TxId`, :class:`Literal`, a plain ``int`` for ``{"$int": "…"}``
    or for integers in safe range, or the scalar directly (``str``, ``int``, ``float``,
    ``bool``).
    """
    if isinstance(v := j, bool):
        return v
    if isinstance(j, (int, float)):
        return j
    if isinstance(j, str):
        return j
    if not isinstance(j, dict):
        return j
    if "iri" in j:
        return Iri(str(j["iri"]))
    if "node" in j:
        return Node(int(j["node"]))
    if "bnode" in j:
        return BNode(int(j["bnode"]))
    if "bnodeLabel" in j:
        # RDF blank node from SPARQL CONSTRUCT / DESCRIBE
        return str(j["bnodeLabel"])
    if "stmt" in j:
        return Stmt(int(j["stmt"]))
    if "tx" in j:
        return TxId(int(j["tx"]))
    if "$int" in j:
        return int(str(j["$int"]))
    if "lex" in j:
        lex, datatype = str(j["lex"]), j.get("datatype")
        if datatype == _XSD + "dateTime":
            parsed = _parse_datetime(lex)
            if parsed is not None:
                return parsed
        elif datatype == _XSD + "date":
            try:
                return date.fromisoformat(lex)
            except ValueError:
                pass
        return Literal(lex, datatype=datatype, lang=j.get("lang"))
    # RDF triple (from SPARQL CONSTRUCT nested triple)
    if "triple" in j:
        t = j["triple"]
        return (term_from_json(t["s"]), term_from_json(t["p"]), term_from_json(t["o"]))
    return j


def time_to_json(v: Union[datetime, date, int, str]) -> Union[int, str]:
    """Convert a time value to the bridge's epoch-ms integer or RFC 3339 string."""
    if isinstance(v, str):
        return v
    if isinstance(v, bool):
        raise TypeError("bool is not a time value")
    if isinstance(v, int):
        return v
    if isinstance(v, datetime):
        # A naive datetime is taken as UTC.
        utc = v.replace(tzinfo=timezone.utc) if v.tzinfo is None else v.astimezone(timezone.utc)
        return int(utc.timestamp() * 1000)
    if isinstance(v, date):
        return int(datetime(v.year, v.month, v.day, tzinfo=timezone.utc).timestamp() * 1000)
    raise TypeError(f"expected a time value, got {type(v).__name__!r}")


def ms_to_datetime(ms: Optional[int]) -> Optional[datetime]:
    """Convert an optional epoch-ms value to a UTC datetime."""
    if ms is None:
        return None
    return datetime.fromtimestamp(ms / 1000.0, tz=timezone.utc)


# --------------------------------------------------------------------------- result types


@dataclass(frozen=True)
class Statement:
    """A statement (triple with metadata) returned by :meth:`View.triples`."""

    eid: int
    s: Any
    p: Any
    o: Any
    t_add: int
    t_ret: Optional[int]
    valid_from: Optional[int]
    valid_to: Optional[int]
    ret_kind: Optional[str]


def statement_from_json(j: Dict[str, Any]) -> Statement:
    """Decode a bridge triple-object into a :class:`Statement`."""
    return Statement(
        eid=int(j["eid"]),
        s=term_from_json(j["s"]),
        p=term_from_json(j["p"]),
        o=term_from_json(j["o"]),
        t_add=int(j["tAdd"]),
        t_ret=None if j.get("tRet") is None else int(j["tRet"]),
        valid_from=None if j.get("validFrom") is None else int(j["validFrom"]),
        valid_to=None if j.get("validTo") is None else int(j["validTo"]),
        ret_kind=j.get("retKind") or None,
    )


@dataclass
class Report:
    """The outcome of a completed transaction."""

    t: int
    """The transaction number (1-based, never skips)."""
    instant: int
    """Wall-clock time of the transaction as epoch-ms."""
    asserted: List[int]
    """Eids of newly inserted statements."""
    existing: List[int]
    """Eids of statements that already existed and were not re-inserted."""
    retracted: List[Dict[str, Any]]
    """Eids and kinds of retracted statements: ``[{"eid": n, "kind": "explicit"|...}]``."""
    superseded: List[Dict[str, Any]]
    """Old/new eid pairs for superseded statements: ``[{"old": n, "new": m}]``."""
    memberships: List[int]
    """Eids of graph-membership statements asserted."""
    memberships_retracted: List[Dict[str, Any]]
    """Eids and kinds of graph-membership statements retracted."""
    results: List[Any] = field(default_factory=list)
    """One result object per op, in order (for ``import_bundle``: ``{"root", "statements"}``)."""


def report_from_json(j: Dict[str, Any]) -> Report:
    """Decode a bridge report object into a :class:`Report`."""
    return Report(
        t=int(j["t"]),
        instant=int(j["instant"]),
        asserted=[int(x) for x in j.get("asserted") or []],
        existing=[int(x) for x in j.get("existing") or []],
        retracted=list(j.get("retracted") or []),
        superseded=list(j.get("superseded") or []),
        memberships=[int(x) for x in j.get("memberships") or []],
        memberships_retracted=list(j.get("membershipsRetracted") or []),
        results=list(j.get("results") or []),
    )


@dataclass(frozen=True)
class CypherResult:
    """The result of a Cypher read query."""

    columns: List[str]
    rows: List[List[Any]]

    def __iter__(self) -> Iterator[Dict[str, Any]]:
        """Iterate as dicts mapping column name to value."""
        for row in self.rows:
            yield dict(zip(self.columns, row))

    def __len__(self) -> int:
        return len(self.rows)


@dataclass(frozen=True)
class CypherWriteResult:
    """The result of a Cypher write statement."""

    columns: List[str]
    rows: List[List[Any]]
    report: Optional[Report]


class SparqlSelectResult:
    """SPARQL SELECT result: iterate to get rows as dicts mapping variable to term."""

    def __init__(
        self,
        vars_: List[str],
        rows: List[Dict[str, Any]],
        provenance: Optional[List[List[Any]]] = None,
    ) -> None:
        self.vars: List[str] = vars_
        self._rows = rows
        self.provenance: Optional[List[List[Stmt]]] = (
            None
            if provenance is None
            else [[term_from_json(e) for e in row] for row in provenance]
        )
        """With ``provenance=True``: the statements behind each row, parallel to the rows."""

    def __iter__(self) -> Iterator[Dict[str, Any]]:
        for row in self._rows:
            yield {k: term_from_json(v) for k, v in row.items()}

    def __len__(self) -> int:
        return len(self._rows)


class SparqlAskResult:
    """SPARQL ASK result."""

    def __init__(self, value: bool) -> None:
        self.value = value

    def __bool__(self) -> bool:
        return self.value


@dataclass(frozen=True)
class SparqlGraphResult:
    """SPARQL CONSTRUCT / DESCRIBE result."""

    triples: List[Dict[str, Any]]


@dataclass(frozen=True)
class SparqlUpdateResult:
    """SPARQL UPDATE result."""

    report: Optional[Report]


SparqlResult = Union[SparqlSelectResult, SparqlAskResult, SparqlGraphResult, SparqlUpdateResult]


def sparql_result_from_json(j: Dict[str, Any]) -> SparqlResult:
    """Decode a bridge SPARQL output dict into the appropriate Python result type."""
    kind = j.get("kind")
    if kind == "select":
        vars_: List[str] = list(j.get("vars") or [])
        rows: List[Dict[str, Any]] = list(j.get("rows") or [])
        return SparqlSelectResult(vars_, rows, j.get("provenance"))
    if kind == "ask":
        return SparqlAskResult(bool(j.get("value", False)))
    if kind == "graph":
        return SparqlGraphResult(list(j.get("triples") or []))
    if kind == "update":
        rep_j = j.get("report")
        return SparqlUpdateResult(report_from_json(rep_j) if rep_j else None)
    raise ValueError(f"unknown SPARQL result kind {kind!r}")


@dataclass(frozen=True)
class PathRow:
    """One row from :meth:`View.path`."""

    start: Any
    end: Any
    hops: int
    path: Optional[Dict[str, Any]]
    arrival: Optional[int] = None
    """Arrival instant (epoch ms) of a time-respecting search, otherwise ``None``."""
