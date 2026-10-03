#!/usr/bin/env bash
# The facade feature matrix (OpenSpec change `add-optional-query-frontends`).
#
#   scripts/feature-matrix.sh <combo> [deps|check|test|handoff|all]
#
# combo: core | exec | sparql | cypher | default | bindings (deps only)
#   deps   assert the normal dependency tree of `tiramemsu` (`cargo tree -e normal`):
#          which query crates and parsers are present and which are absent
#   check  `cargo check` of the library, tests and examples in this combination
#   test   `cargo test -p tiramemsu` in this combination (the smoke tests in
#          tests/feature_matrix.rs run in every one)
#   handoff  a database file written here is read by the default build, and
#          the reverse
#   all    deps, check, test, then handoff (the default)
set -euo pipefail

combo="${1:?usage: feature-matrix.sh <core|exec|sparql|cypher|default|bindings> [deps|check|test|handoff|all]}"
step="${2:-all}"
packages=(tiramemsu)

case "$combo" in
  core)
    flags=(--no-default-features)
    present=(tm-core tm-rusqlite)
    absent=(tm-ir tm-exec tm-sparql tm-cypher spargebra peg open-cypher)
    ;;
  exec)
    flags=(--no-default-features --features exec)
    present=(tm-core tm-rusqlite tm-ir tm-exec)
    absent=(tm-sparql tm-cypher spargebra peg open-cypher)
    ;;
  sparql)
    flags=(--no-default-features --features sparql)
    present=(tm-core tm-rusqlite tm-ir tm-exec tm-sparql spargebra)
    absent=(tm-cypher open-cypher)
    ;;
  cypher)
    flags=(--no-default-features --features cypher)
    present=(tm-core tm-rusqlite tm-ir tm-exec tm-cypher open-cypher)
    absent=(tm-sparql spargebra peg)
    ;;
  default)
    flags=()
    present=(tm-core tm-rusqlite tm-ir tm-exec tm-sparql spargebra tm-cypher open-cypher)
    absent=()
    ;;
  bindings)
    # the JSON bridge, the MCP server and the Node and Python bindings ship the
    # full facade: their trees must hold both front ends
    flags=()
    present=(tm-core tm-rusqlite tm-ir tm-exec tm-sparql spargebra tm-cypher open-cypher)
    absent=()
    packages=(tiramemsu-json tiramemsu-mcp tiramemsu-node tiramemsu-python)
    ;;
  *)
    echo "unknown combo: $combo" >&2
    exit 2
    ;;
esac

deps() {
  local pkg
  for pkg in "${packages[@]}"; do
    deps_of "$pkg"
  done
}

deps_of() {
  local pkg="$1" tree
  # one line per crate, deduplicated, without the tree drawing
  tree="$(cargo tree -p "$pkg" -e normal --prefix none ${flags[@]+"${flags[@]}"} | awk '{print $1}' | sort -u)"
  local fail=0
  for c in ${present[@]+"${present[@]}"}; do
    if ! grep -qx "$c" <<<"$tree"; then
      echo "[$combo] $pkg: expected dependency missing: $c" >&2
      fail=1
    fi
  done
  for c in ${absent[@]+"${absent[@]}"}; do
    if grep -qx "$c" <<<"$tree"; then
      echo "[$combo] $pkg: forbidden dependency present: $c" >&2
      cargo tree -p "$pkg" -e normal ${flags[@]+"${flags[@]}"} -i "$c" >&2 || true
      fail=1
    fi
  done
  if [ "$fail" -ne 0 ]; then
    exit 1
  fi
  echo "[$combo] $pkg: dependency tree ok (present: ${present[*]-none}; absent: ${absent[*]-none})"
}

check() {
  if [ "$combo" = bindings ]; then
    return
  fi
  cargo check -p tiramemsu --lib --tests --examples ${flags[@]+"${flags[@]}"}
}

run_tests() {
  if [ "$combo" = bindings ]; then
    return
  fi
  cargo test -p tiramemsu ${flags[@]+"${flags[@]}"}
}

# One database file written by this combination and read by the default build,
# then the reverse: the persisted format does not depend on the features.
handoff() {
  if [ "$combo" = bindings ]; then
    return
  fi
  local dir
  dir="$(mktemp -d)"
  trap 'rm -rf "$dir"' RETURN
  TIRAMEMSU_HANDOFF="$dir/a.db" cargo test -p tiramemsu ${flags[@]+"${flags[@]}"} --test feature_matrix handoff
  TIRAMEMSU_HANDOFF="$dir/a.db" cargo test -p tiramemsu --test feature_matrix handoff
  TIRAMEMSU_HANDOFF="$dir/b.db" cargo test -p tiramemsu --test feature_matrix handoff
  TIRAMEMSU_HANDOFF="$dir/b.db" cargo test -p tiramemsu ${flags[@]+"${flags[@]}"} --test feature_matrix handoff
  echo "[$combo] database file handed to and from the default build"
}

case "$step" in
  deps) deps ;;
  check) check ;;
  test) run_tests ;;
  handoff) handoff ;;
  all)
    deps
    check
    run_tests
    handoff
    ;;
  *)
    echo "unknown step: $step" >&2
    exit 2
    ;;
esac
