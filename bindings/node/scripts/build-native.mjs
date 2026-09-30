// Builds the native addon with cargo and copies it to tiramemsu.node.
import { execFileSync } from "node:child_process";
import { copyFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../../..", import.meta.url));
const profile = process.argv.includes("--debug") ? "debug" : "release";
execFileSync(
  "cargo",
  ["build", "-p", "tiramemsu-node", ...(profile === "release" ? ["--release"] : [])],
  { cwd: root, stdio: "inherit" },
);
const file = { darwin: "libtiramemsu_node.dylib", win32: "tiramemsu_node.dll" }[process.platform] ?? "libtiramemsu_node.so";
const built = `${root}/target/${profile}/${file}`;
copyFileSync(built, fileURLToPath(new URL("../tiramemsu.node", import.meta.url)));
// The published package carries one prebuilt binary per platform.
copyFileSync(built, fileURLToPath(new URL(`../tiramemsu.${process.platform}-${process.arch}.node`, import.meta.url)));
