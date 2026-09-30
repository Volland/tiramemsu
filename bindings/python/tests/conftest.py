"""pytest fixtures shared across the test suite."""

import pytest

from tiramemsu import Database, Iri


@pytest.fixture
def db(tmp_path):
    """A fresh in-memory-equivalent database in a temp file."""
    return Database(str(tmp_path / "test.db"))


@pytest.fixture
def alice_db(tmp_path):
    """Database with the standard Alice/Acme/Globex time-travel story loaded.

    Returns ``(db, db_path, t2_report)`` where ``t2_report`` is the report from
    tx2 (the supersede transaction).
    """
    path = str(tmp_path / "alice.db")
    database = Database(path)

    alice = Iri("urn:tiramemsu:v:alice")
    works_at = Iri("urn:tiramemsu:v:worksAt")
    acme = Iri("urn:tiramemsu:v:acme")
    globex = Iri("urn:tiramemsu:v:globex")
    confidence = Iri("urn:tiramemsu:v:confidence")

    # tx1: alice worksAt acme valid_from 2020-01-01, with confidence layer
    with database.transact() as tx:
        job = tx.assert_(alice, works_at, acme, valid_from="2020-01-01")
        tx.assert_(job, confidence, 0.8)

    # tx2: supersede with valid_to 2024-01-01
    t2_ctx = database.transact()
    with t2_ctx as tx:
        tx.supersede(1, valid_to="2024-01-01")
    t2_report = t2_ctx._builder.report

    # tx3: alice worksAt globex valid_from 2024-03-01
    with database.transact() as tx:
        tx.assert_(alice, works_at, globex, valid_from="2024-03-01")

    return database, path, t2_report
