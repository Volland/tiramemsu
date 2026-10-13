#!/usr/bin/env python3
"""Package the validation archive for the Zenodo record (second file next to the PDF).

Refuses stale validation, builds the Lean project, then zips the manuscript source,
the executable validation, its results, the engine trace and test, and the Lean
development, with SHA-256 checksums. Output: zenodo/<NAME>.zip
"""
from pathlib import Path
import hashlib
import json
import subprocess
import tempfile
import zipfile

PAPER = Path(__file__).resolve().parent.parent
REPO = PAPER.parent.parent
NAME = "statement-as-one-record-r4-validation"

FILES = [
    "statement-as-one-record.tex", "references.bib", "validation-counts.tex",
    "statement-as-one-record.pdf", "validate.py", "validation-results.json",
    "engine-trace.json", "build.py", "SOURCE_AUDIT.md", "REVISION_NOTES.md",
]
LEAN = ["lean/README.md", "lean/lakefile.toml", "lean/lake-manifest.json",
        "lean/lean-toolchain", "lean/StatementExport.lean", "lean/Axioms.lean"]
ENGINE_TEST = "crates/tm-core/tests/export_boundary.rs"

README = """# Validation archive: When Is a Temporal Statement Store a Statement Graph?

Revision 4, 12 October 2026. Volodymyr Pavlyshyn. Companion to the PDF in the same Zenodo
record. Source repository: https://github.com/Volland/tiramemsu
(paper/statement-as-one-record/).

## Contents

- `paper/`: LaTeX source, bibliography, the PDF, `build.py`, the source audit and
  revision notes, and the executable validation `validate.py` (Python 3, standard
  library only). The validator covers exhaustive stores of up to three records, all
  four-record reference structures up to relabeling, random stores and random protocol
  histories, with and without the acyclic-replay rule. It writes
  `validation-results.json` and the manuscript counts `validation-counts.tex`.
- `paper/engine-trace.json`: written by the tiramemsu engine test
  `engine/export_boundary.rs` (tm-core 0.4.0) with `TM_EXPORT_TRACE` set. The
  validator recomputes the kernels from it independently.
- `lean/`: Lean 4 mechanization of the paper's combinatorial results (core only, no
  Mathlib). `lean/README.md` maps each theorem label to its Lean name and lists what
  is not mechanized.
- `SHA256SUMS`: checksums of every file above.

## Reproduce

    cd paper && python3 validate.py && python3 build.py   # ~1 minute; build needs pdflatex
    cd lean && lake build && lake env lean Axioms.lean

`validate.py` is deterministic. Its results record the SHA-256 of the script and of
the engine trace. Regenerating the trace needs the full repository:

    TM_EXPORT_TRACE=$PWD/paper/statement-as-one-record cargo test -p tm-core --test export_boundary

## Limits

- The PDF's proofs are by hand. The Lean development re-proves the closure, kernel,
  history, encoding, query-view and reifier results under the modelling choices in
  `lean/README.md`. The LPG proposition and the algorithmic cost bounds are not
  mechanized.
- Validation is finite. Engine evidence is limited to the two traced examples.
- No performance claims are made.
"""


def sha(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    evidence = json.loads((PAPER / "validation-results.json").read_text())
    if evidence["script_sha256"] != sha(PAPER / "validate.py"):
        raise SystemExit("Validation is stale: run python3 validate.py first.")
    if evidence["engine_trace"]["sha256"] != sha(PAPER / "engine-trace.json"):
        raise SystemExit("Engine trace changed since validation: rerun validate.py.")
    build = subprocess.run(["lake", "build"], cwd=PAPER / "lean", text=True,
                           stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    if build.returncode or "sorry" in build.stdout:
        raise SystemExit("Lean build failed:\n" + build.stdout)

    entries = {}  # archive path -> source path
    for name in FILES:
        entries[f"paper/{name}"] = PAPER / name
    for name in LEAN:
        entries[name] = PAPER / name
    for path in sorted((PAPER / "lean" / "StatementExport").glob("*.lean")):
        entries[f"lean/StatementExport/{path.name}"] = path
    entries["engine/export_boundary.rs"] = REPO / ENGINE_TEST

    with tempfile.TemporaryDirectory() as tmp:
        readme = Path(tmp) / "README.md"
        readme.write_text(README)
        entries = {"README.md": readme, **entries}
        sums = "".join(f"{sha(src)}  {arc}\n" for arc, src in entries.items())
        out = PAPER / "zenodo" / f"{NAME}.zip"
        with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as archive:
            for arc, src in entries.items():
                info = zipfile.ZipInfo(f"{NAME}/{arc}", date_time=(2026, 10, 12, 0, 0, 0))
                info.compress_type = zipfile.ZIP_DEFLATED
                archive.writestr(info, src.read_bytes())
            info = zipfile.ZipInfo(f"{NAME}/SHA256SUMS", date_time=(2026, 10, 12, 0, 0, 0))
            archive.writestr(info, sums)
    print(f"Wrote {out} ({len(entries) + 1} files), sha256 {sha(out)}")


if __name__ == "__main__":
    main()
