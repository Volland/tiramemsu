"""Run the reduced triangle experiment from the repository root."""
from pathlib import Path

namespace = {}
source = Path("bench/triangles/bench.py").read_text()
exec(source.split("N=100_000")[0], namespace)
print("SQLite version:", namespace["sqlite3"].sqlite_version)
for size in (25, 50, 100):
    layer_a = list(range(1, size + 1))
    layer_b = list(range(10001, 10001 + size))
    layer_c = list(range(20001, 20001 + size))
    edges = (
        [(a, b) for a in layer_a for b in layer_b]
        + [(b, c) for b in layer_b for c in layer_c]
        + [(c, a) for c in layer_c for a in namespace["random"].sample(layer_a, 3)]
    )
    namespace["run"](f"layered n={size}", edges)
