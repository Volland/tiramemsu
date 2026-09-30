// Checks a vitest JSON report: every test must pass.
import { readFileSync } from "node:fs";

const report = JSON.parse(readFileSync(process.argv[2] ?? "test-results.json", "utf8"));
const tests = report.testResults.flatMap((f) => f.assertionResults);
const passed = tests.filter((t) => t.status === "passed").length;
const failed = tests.filter((t) => t.status === "failed");
for (const t of failed) {
  const id = [...t.ancestorTitles, t.title].join(" ");
  console.error(`FAIL: ${id}\n  ${t.failureMessages?.[0]?.split("\n")[0] ?? ""}`);
}
const ok = report.numTotalTests > 0 && failed.length === 0;
console.log(`${passed}/${tests.length} tests passed`);
process.exit(ok ? 0 : 1);
