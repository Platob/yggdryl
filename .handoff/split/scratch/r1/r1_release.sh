#!/usr/bin/env bash
# r1_release.sh - bump a release candidate on a branch, or prove one, publishing nothing.
#
#   r1_release.sh <version>   move every manifest from the current version to <version>, then stop:
#                             commit the bump, then run --prove on the committed tree
#   r1_release.sh --prove     prove the current version as the tree stands (the first release's
#                             proof for 0.1.22, which is already bumped)
#
# Both refuse `main` and a detached HEAD; --prove refuses a dirty tree, because the release's
# `sources` job proves a commit and cargo's VCS check wants one (no --allow-dirty here). The bump
# reads crates.io, PyPI and npm read-only (an unreachable registry stops it) and moves the seven
# files AGENTS.md section 6 names; the proof runs the dry runs the release's `sources` job runs.
# It never runs `cargo publish` without `--dry-run`, never pushes, never tags, never edits a
# workflow: release.yml is the lane's own commit. Run it from the repository root on the program
# branch with python/.venv and node/node_modules in place; it builds the debug addon itself.
# Outputs land in $R1_OUT, else a fresh temporary directory it prints - never in the repository.
set -euo pipefail

ROOT=$(git rev-parse --show-toplevel)
cd "$ROOT"
PY=python/.venv/bin/python
OUT=${R1_OUT:-$(mktemp -d "${TMPDIR:-/tmp}/yggdryl-r1.XXXXXX")}
mkdir -p "$OUT"
printf 'outputs under %s\n' "$OUT"

die() { printf 'r1_release: %s\n' "$*" >&2; exit 1; }
step() { printf '\n== %s\n' "$*"; }

[ $# -eq 1 ] || die "usage: r1_release.sh <version> | --prove"
branch=$(git branch --show-current)
[ -n "$branch" ] || die "refusing to run on a detached HEAD: check the program branch out"
[ "$branch" != main ] || die "refusing to run on main: the merge is the release"
[ -x "$PY" ] || die "python/.venv is missing (see MARKET_SPLIT_CONTINUE.md, Setup)"
[ -d node/node_modules ] || die "node/node_modules is missing: npm ci --prefix node"

current=$(python3 -I scripts/release_packages.py version)
crates=$(python3 -I scripts/release_packages.py crates)
if [ "$1" = --prove ]; then
  [ -z "$(git status --porcelain)" ] || die "the tree is dirty; commit first - the proof is of a commit"
  target=$current
else
  target=$1
  [[ $target =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "not a version: $target"
  [ "$target" != "$current" ] || die "$target is already the tree's version; use --prove"
  [ "$(printf '%s\n' "$current" "$target" | sort -V | tail -n 1)" = "$target" ] \
    || die "$target is below the tree's $current; a released version is never reused"
fi
printf 'current %s, target %s, crates: %s\n' "$current" "$target" "$crates"

# A registry answers 200 (held) or 404 (free); anything else means it was not reached, and a
# release decided on a registry nobody reached is one decided blind. Proxy settings come from
# the environment (HTTPS_PROXY, the CA bundle in CURL_CA_BUNDLE or the proxy's own).
held=0
probe() { # probe <label> <url> [<grep for a 200 body>]
  local code body
  body=$(mktemp "$OUT/probe.XXXXXX")
  code=$(curl -sS --max-time 30 -o "$body" -w '%{http_code}' -A 'yggdryl-release-check' "$2") \
    || die "cannot reach $1 ($2)"
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
# Every workspace package of the lock carries <version>, or the lock was not moved with the tree.
lock_carries() { # lock_carries <version>
  python3 -I - "$1" <<'EOF'
import sys, tomllib, pathlib
version = sys.argv[1]
lock = tomllib.loads(pathlib.Path("Cargo.lock").read_text(encoding="utf-8"))
ours = {p["name"]: p["version"] for p in lock["package"] if "source" not in p}
bad = {k: v for k, v in ours.items() if v != version}
if not ours or bad:
    sys.exit(f"Cargo.lock: workspace packages not at {version}: {bad or 'none found'}")
print(f"  Cargo.lock: {len(ours)} workspace packages at {version}")
EOF
}
if [ "$target" != "$current" ]; then
  step "the seven files move to $target"
  pins=$(grep -cF "version = \"=$current\"" Cargo.toml || true)                                  # the workspace pins, counted from the tree
  [ "$pins" -ge 1 ] || die "Cargo.toml: no workspace pin spelled =$current"
  replace Cargo.toml "version = \"$current\"" "version = \"$target\"" 1                           # [workspace.package]
  replace Cargo.toml "version = \"=$current\"" "version = \"=$target\"" "$pins"
  replace python/pyproject.toml "version = \"$current\"" "version = \"$target\"" 1
  npm --prefix node version "$target" --no-git-tag-version --allow-same-version >/dev/null         # package.json + package-lock.json
  if [ -f python/market/pyproject.toml ]; then                                                    # B5's manifests, once they exist
    replace python/market/pyproject.toml "version = \"$current\"" "version = \"$target\"" 1
    replace python/market/pyproject.toml "yggdryl==$current" "yggdryl==$target" "$(grep -cF "yggdryl==$current" python/market/pyproject.toml || true)"
  fi
  if [ -f node/market/package.json ]; then
    npm --prefix node/market version "$target" --no-git-tag-version --allow-same-version >/dev/null
  fi
  cargo update --workspace                                                                        # Cargo.lock, nothing else
  lock_carries "$target"
  npm run --prefix node build:debug                                                              # the addon the manifests read
  node scripts/build_docs_fix.js && node scripts/build_docs_playground.js                        # docs/assets/{fix,playground}.json
  node scripts/build_docs_fix.js --check && node scripts/build_docs_playground.js --check
  [ "$(python3 -I scripts/release_packages.py version)" = "$target" ] || die "the manifests disagree after the bump"
  git diff --stat
  cat <<EOF

Bumped $current -> $target in the tree; nothing published, nothing committed. Next, the lane's:
  1. commit this bump on the branch: one commit, ending with exactly these two lines and no other
     Co-Authored-By (the program's standing rule, .handoff/next/MARKET_SPLIT_CONTINUE.md):
       Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
       Claude-Session: https://claude.ai/code/session_01Gfky7FUx35U5i4UJcrQGKp
  2. run '$0 --prove' on the committed tree, then push and read CI to 'CI result'.
EOF
  exit 0
fi

step "the addon the audits and the manifests read"
npm run --prefix node build:debug
node scripts/build_docs_fix.js --check && node scripts/build_docs_playground.js --check
lock_carries "$target"

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

# Listings are written to files and grepped there: under pipefail, `producer | grep -q` dies of
# SIGPIPE in the producer once grep has its match, and a found entry reads as a failure.
step "the wheel builds (CI's wheel job's profile) and carries the command"
rm -rf "$OUT/dist"
python3 scripts/stage_cli.py --debug --locked
VIRTUAL_ENV=python/.venv "$PY" -m maturin build --locked --profile dev --manifest-path python/Cargo.toml --interpreter "$PY" --out "$OUT/dist"
wheels=$(compgen -G "$OUT/dist/yggdryl-$target-*.whl" || true)
[ -n "$wheels" ] || die "no wheel yggdryl-$target-*.whl was built under $OUT/dist"
wheel=$(printf '%s\n' "$wheels" | head -n 1)
unzip -l "$wheel" > "$OUT/wheel.files"
grep -q "yggdryl-$target.data/scripts/yggdryl" "$OUT/wheel.files" || die "the wheel carries no yggdryl command"
VIRTUAL_ENV=python/.venv "$PY" -m maturin sdist --manifest-path python/Cargo.toml --out "$OUT/dist"
sdist="$OUT/dist/yggdryl-$target.tar.gz"
[ -f "$sdist" ] || die "no sdist $sdist was built"
tar tzf "$sdist" > "$OUT/sdist.files"
grep -qE '/rust/market/Cargo.toml$' "$OUT/sdist.files" || die "the sdist lacks rust/market"
grep -qE '/rust/fix/Cargo.toml$' "$OUT/sdist.files" || die "the sdist lacks rust/fix"
printf '  wheel %s\n  sdist %s files\n' "$(basename "$wheel")" "$(wc -l <"$OUT/sdist.files")"

step "the npm package audits"
npm run --prefix node test:package:files
(cd node && npm pack --dry-run)

step "what changed"
git diff --stat
cat <<EOF

Proved $target with nothing published; listings under $OUT.
The lane's: push the branch, read CI to 'CI result', record this log in the results commit (which
ends with the same two attribution lines every commit of the program carries).
The user's alone:
  1. CARGO_REGISTRY_TOKEN with publish-new over every crate crates.io does not hold yet: $crates;
  2. merge the PR into main once every gate is green - the merge is the release (preflight publishes
     an untagged version); never tag by hand, never run release.yml by hand except as a rehearsal.
EOF
