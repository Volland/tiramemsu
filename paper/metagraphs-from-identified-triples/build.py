#!/usr/bin/env python3
"""Build the paper in a temporary directory, keeping generated files out of source."""
from pathlib import Path
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parent
STEM = "metagraphs-layered-graphs"


def main():
    with tempfile.TemporaryDirectory(prefix="metagraph-paper-build-") as directory:
        work = Path(directory)
        for name in (STEM + ".tex", "references.bib", "validation-counts.tex", "ablation-table.tex"):
            shutil.copy2(ROOT / name, work / name)
        latex = ["pdflatex", "-interaction=nonstopmode", "-halt-on-error", STEM + ".tex"]
        for command in (latex, ["bibtex", STEM], latex, latex):
            result = subprocess.run(command, cwd=work, text=True,
                                    stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
            if result.returncode:
                raise SystemExit(result.stdout)
        log = (work / (STEM + ".log")).read_text()
        if "undefined" in log.lower() or "Overfull" in log:
            raise SystemExit("PDF validation failed:\n" + log)
        shutil.copy2(work / (STEM + ".pdf"), ROOT / (STEM + ".pdf"))
        for line in log.splitlines():
            if "Output written" in line or "Underfull" in line:
                print(line)
    print("Built " + str(ROOT / (STEM + ".pdf")))


if __name__ == "__main__":
    main()
