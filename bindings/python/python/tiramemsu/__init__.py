"""tiramemsu: a bitemporal, never-forget triple store on SQLite.

Supports SPARQL 1.1 and openCypher queries, layered statements with ids, and
time travel across both transaction time and valid time.

Quick start::

    from tiramemsu import Database, Iri

    db = Database("data.db")

    alice = Iri("urn:ex:alice")
    works_at = Iri("urn:ex:worksAt")
    acme = Iri("urn:ex:acme")

    with db.transact() as tx:
        tx.assert_(alice, works_at, acme, valid_from="2020-01-01")

    for row in db.now().sparql("SELECT ?o WHERE { <urn:ex:alice> <urn:ex:worksAt> ?o }"):
        print(row["o"])
"""

from importlib.metadata import PackageNotFoundError, version

from ._db import BulkImport, Database, Ref, TxBuilder, TxContext, View
from ._types import (
    BNode,
    BundlePreview,
    Conflict,
    ConflictEvidence,
    ConflictValue,
    CypherResult,
    CypherWriteResult,
    ImportProgress,
    ImportSummary,
    Invalidation,
    Iri,
    Literal,
    Node,
    PathCompleteness,
    PathReport,
    PathRow,
    QueryBudget,
    Report,
    SavedAnswer,
    SparqlAskResult,
    SparqlGraphResult,
    SparqlResult,
    SparqlSelectResult,
    SparqlUpdateResult,
    Statement,
    Stmt,
    Term,
    TextEvidence,
    TextHit,
    TiramemsuError,
    TxId,
    ms_to_datetime,
    report_from_json,
    sparql_result_from_json,
    statement_from_json,
    term_from_json,
    term_to_json,
    time_to_json,
)

try:
    __version__: str = version("tiramemsu")
except PackageNotFoundError:  # pragma: no cover - running from a source tree
    __version__ = "0.0.0"

__all__ = [
    "BNode",
    "BundlePreview",
    "Conflict",
    "ConflictEvidence",
    "ConflictValue",
    "BulkImport",
    "CypherResult",
    "CypherWriteResult",
    "Database",
    "ImportProgress",
    "ImportSummary",
    "Invalidation",
    "Iri",
    "Literal",
    "Node",
    "PathCompleteness",
    "PathReport",
    "PathRow",
    "QueryBudget",
    "Ref",
    "Report",
    "SavedAnswer",
    "SparqlAskResult",
    "SparqlGraphResult",
    "SparqlResult",
    "SparqlSelectResult",
    "SparqlUpdateResult",
    "Statement",
    "Stmt",
    "Term",
    "TextEvidence",
    "TextHit",
    "TiramemsuError",
    "TxBuilder",
    "TxContext",
    "TxId",
    "View",
    "__version__",
    "ms_to_datetime",
    "report_from_json",
    "sparql_result_from_json",
    "statement_from_json",
    "term_from_json",
    "term_to_json",
    "time_to_json",
]
