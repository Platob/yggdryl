#!/usr/bin/env bash
# r1_release.sh - bump a release candidate on a branch and prove it, publishing nothing.
#
#   r1_release.sh <version>   bump every manifest from the current version to <version>, then prove
#   r1_release.sh --prove     prove the current version as the tree stands (no bump)
#
# It refuses `main` and a dirty tree, reads crates.io, PyPI and npm read-only (an unreachable
# registry stops it), moves the seven files AGENTS.md section 6 names, and runs the dry runs the
# release's `sources` job runs. It never runs `cargo publish` without `--dry-run`, never pushes,
# never tags, never edits a workflow: release.yml is the lane's own commit. Run it from the
# repository root on the program branch with python/.venv and node/node_modules in place.
set -euo pipefail

ROOT=$(git rev-parse --show-toplevel)
cd "$ROOT"
PY=python/.venv/bin/python
OUT=${S:-/tmp/s}/r1
mkdir -p "$OUT"

die() { printf 'r1_release: %s\n' "$*" >&2; exit 1; }
step() { printf '\n== %s\n' "$*"; }

[ $# -eq 1 ] || die "usage: r1_release.sh <version> | --prove"
[ "$(git branch --show-current)" != main ] || die "refusing to run on main: the merge is the release"
[ -z "$(git status --porcelain)" ] || die "the tree is dirty; commit or stash first"
[ -x "$PY" ] || die "python/.venv is missing (see MARKET_SPLIT_CONTINUE.md, Setup)"

current=$(python3 -I scripts/release_packages.py version)
crates=$(python3 -I scripts/release_packages.py crates)
if [ "$1" = --prove ]; then
  target=$current
else
  target=$1
  [[ $target =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "not a version: $target"
  [ "$target" != "$current" ] || die "$target is already the tree's version; use --prove"
fi
printf 'current %s, target %s, crates: %s\n' "$current" "$target" "$crates"

# A registry answers 200 (held) or 404 (free); anything else means it was not reached, and a
# release decided on a registry nobody reached is one decided blind. Proxy settings come from
# the environment (HTTPS_PROXY, the CA bundle in CURL_CA_BUNDLE or the proxy's own).
held=0
probe() { # probe <label> <url> [<grep for a 200 body>]
  local code body
  body=$(mktemp "$OUT/probe.XXXXXX")
  code=$(curl -sS -o "$body" -w '%{http_code}' -A 'yggdryl-release-check' "$2") || die "cannot reach $1 ($2)"
  case $code in
    200) if [ -z "${3:-}" ] || grep -q -- "$3" "$body"; then printf '  %-28s held\n' "$1"; held=1; else printf '  %-28s free\n' "$1"; fi ;;
    404) printf '  %-28s free\n' "$1" ;;
    *) die "$1 answered HTTP $code ($2)" ;;
  esac
  rm -f "$body"
}
step "the registries must not carry $target"
for crate in $crates; do
  probe "crates.io $crate" "https://index.crates.io/${crate:0:2}/${crate:2:2}/$crate" "\"vers\":\"$target\""
done
for name in $(python3 -I scripts/release_packages.py pypi); do probe "pypi $name" "https://pypi.org/pypi/$name/$target/json"; done
for name in $(python3 -I scripts/release_packages.py npm); do probe "npm $name" "https://registry.npmjs.org/$name/$target"; done
[ $held -eq 0 ] || die "$target is already on a registry; a released version is never reused - pick the next one"

# One exact edit per anchor, each asserted to match the expected number of times, so a manifest
# whose shape moved is a refusal and never a silent half-bump.
replace() { # replace <file> <old> <new> <expected count>
  local n
  n=$(grep -cF -- "$2" "$1" || true)
  [ "$n" -eq "$4" ] || die "$1: expected $4 of '$2', found $n"
  python3 -I - "$1" "$2" "$3" <<'EOF'
import sys, pathlib
p = pathlib.Path(sys.argv[1]); p.write_text(p.read_text(encoding="utf-8").replace(sys.argv[2], sys.argv[3]), encoding="utf-8")
EOF
}
if [ "$target" != "$current" ]; then
  step "the seven files move to $target"
  replace Cargo.toml "version = \"$current\"" "version = \"$target\"" 1                       # [workspace.package]
  replace Cargo.toml "version = \"=$current\"" "version = \"=$target\"" "$(wc -w <<<"$crates")" # the workspace pins
  replace python/pyproject.toml "version = \"$current\"" "version = \"$target\"" 1
  npm --prefix node version "$target" --no-git-tag-version --allow-same-version >/dev/null         # package.json + package-lock.json
  cargo update --workspace                                                                        # Cargo.lock, nothing else
  npm run --prefix node build:debug                                                              # the addon the manifests read
  node scripts/build_docs_fix.js && node scripts/build_docs_playground.js                        # docs/assets/{fix,playground}.json
  [ "$(python3 -I scripts/release_packages.py version)" = "$target" ] || die "the manifests disagree after the bump"
fi

step "every crate packages and verifies from its package alone (the release's sources step, verbatim)"
packages=(); for crate in $crates; do packages+=(-p "$crate"); done
cargo publish --locked --dry-run "${packages[@]}"
for crate in $crates; do
  cargo package --locked --list -p "$crate" > "$OUT/$crate.files"
  printf '  %s: %s files\n' "$crate" "$(wc -l <"$OUT/$crate.files")"
done
! grep -qE '^(market|fix)/' "$OUT/yggdryl.files" || die "the core's package carries market/ or fix/"
grep -q '^README.md$' "$OUT/yggdryl-market.files" || die "yggdryl-market packages no README.md"
grep -q '^README.md$' "$OUT/yggdryl-fix.files" || die "yggdryl-fix packages no README.md"

step "the wheel builds and carries the command"
rm -rf "$OUT/dist"
python3 scripts/stage_cli.py --debug --locked
VIRTUAL_ENV=python/.venv "$PY" -m maturin build --locked --manifest-path python/Cargo.toml --interpreter "$PY" --out "$OUT/dist"
wheel=$(ls "$OUT"/dist/yggdryl-"$target"-*.whl | head -n 1)
unzip -l "$wheel" | grep -q "yggdryl-$target.data/scripts/yggdryl" || die "the wheel carries no yggdryl command"
VIRTUAL_ENV=python/.venv "$PY" -m maturin sdist --manifest-path python/Cargo.toml --out "$OUT/dist"
tar tzf "$OUT/dist/yggdryl-$target.tar.gz" | grep -qE '/rust/market/Cargo.toml$' || die "the sdist lacks rust/market"
tar tzf "$OUT/dist/yggdryl-$target.tar.gz" | grep -qE '/rust/fix/Cargo.toml$' || die "the sdist lacks rust/fix"

step "the npm package audits"
npm run --prefix node test:package:files
(cd node && npm pack --dry-run)

step "what changed"
git diff --stat
cat <<EOF

Proved $target with nothing published. What remains is the user's alone:
  1. commit this bump on the branch (one commit, the two trailer lines), push, read CI to 'CI result';
  2. CARGO_REGISTRY_TOKEN with publish-new over every crate crates.io does not hold yet: $crates;
  3. merge the PR into main once every gate is green - the merge is the release (preflight publishes
     an untagged version); never tag by hand, never run release.yml by hand except as a rehearsal.
EOF
