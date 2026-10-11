#!/usr/bin/env python3
"""Build the paper in a temporary directory, keeping generated files out of source."""
from pathlib import Path
import hashlib
import json
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parent
STEM = "statement-as-one-record"


def main():
    evidence = json.loads((ROOT / "validation-results.json").read_text())
    digest = hashlib.sha256((ROOT / "validate.py").read_bytes()).hexdigest()
    if evidence["script_sha256"] != digest:
        raise SystemExit("Validation is stale: run python3 validate.py first.")
    with tempfile.TemporaryDirectory(prefix="statement-paper-build-") as directory:
        work = Path(directory)
        for name in (STEM + ".tex", "references.bib", "validation-counts.tex"):
            shutil.copy2(ROOT / name, work / name)
        latex = ["pdflatex", "-interaction=nonstopmode", "-halt-on-error", STEM + ".tex"]
        for command in (latex, ["bibtex", STEM], latex, latex):
            result = subprocess.run(command, cwd=work, text=True,
                                    stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
            if result.returncode:
                raise SystemExit(result.stdout)
        log = (work / (STEM + ".log")).read_text()
        if "undefined" in log.lower() or "Overfull" in log:
            lines = log.splitlines()
            issues = ["\n".join(lines[max(0, i-1):i+6]) for i, line in enumerate(lines)
                      if "undefined" in line.lower() or "Overfull" in line]
            raise SystemExit("PDF validation failed:\n" + "\n".join(issues))
        shutil.copy2(work / (STEM + ".pdf"), ROOT / (STEM + ".pdf"))
        for line in log.splitlines():
            if "Output written" in line or "Underfull" in line:
                print(line)
    print("Built " + str(ROOT / (STEM + ".pdf")))


if __name__ == "__main__":
    main()
