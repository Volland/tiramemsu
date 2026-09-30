"""Stub for the native extension module ``tiramemsu._native``."""

class Native:
    """The native database handle: opens a tiramemsu database and runs operations on it."""

    def __new__(cls, path: str, options: str | None = None) -> "Native":
        """Open (or create) the database at *path*.

        *options* is a JSON object string with open options, or ``None``.
        Raises ``RuntimeError`` whose message starts with ``tiramemsu:`` on failure.
        """
        ...

    def call(self, op: str, args: str) -> str:
        """Run one operation on the database.

        *args* is a JSON object string, or an empty string for operations with no
        arguments.  Returns a JSON string.  Raises ``RuntimeError`` whose message
        is ``tiramemsu:<json>`` where ``<json>`` is ``{"code","message"}`` on failure.
        """
        ...
