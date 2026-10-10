#!/usr/bin/env python3
"""Every package the release publishes, the one version they all carry, and the order crates.io takes them.

The release publishes one version everywhere or nowhere: every crate of the
workspace to crates.io, `yggdryl` to PyPI and to npm.
This reads the tree once and answers what `release.yml` needs, so the
workflow spells no package name and no order of its own:

- `version`: the one version, after checking that every manifest carries it -
  the workspace, every published crate (`version.workspace = true`), every
  workspace crate's pin in `[workspace.dependencies]` (`=<version>`), the
  Python project and the npm package with its lock, and any market package's
  pin of the core (`yggdryl==<version>`, the exact peer `"yggdryl": "<version>"`).
  A disagreement is the refusal, naming each manifest and what it says.
- `crates`: every published crate, space-separated, in the order crates.io
  takes them - a crate after every workspace crate it names, a
  dev-dependency included, because crates.io refuses a crate naming one it
  does not hold; ties in the order the workspace lists its members.
- `pypi`, `npm`: the names those registries carry.

`ci.yml`'s inventory job runs `version` on every pull request, so a manifest a
bump forgot fails there rather than in the release.

Standard library only.

Usage:
    python3 scripts/release_packages.py version
    python3 scripts/release_packages.py crates
"""

from __future__ import annotations

import json
import pathlib
import sys
import tomllib

ROOT = pathlib.Path(__file__).resolve().parent.parent
PYPI = ["yggdryl"]
NPM = ["yggdryl"]
SECTIONS = ("dependencies", "dev-dependencies", "build-dependencies")


def toml(path: pathlib.Path) -> dict:
    with path.open("rb") as handle:
        return tomllib.load(handle)


def members() -> list[tuple[str, dict]]:
    workspace = toml(ROOT / "Cargo.toml")["workspace"]
    out = []
    for member in workspace.get("members", []):
        manifest = toml(ROOT / member / "Cargo.toml")
        out.append((member, manifest))
    return out


def published() -> list[tuple[str, dict]]:
    return [
        (member, manifest)
        for member, manifest in members()
        if manifest.get("package", {}).get("publish", True) is not False
    ]


def dependency_names(manifest: dict) -> set[str]:
    names: set[str] = set()
    tables = [manifest.get(section, {}) for section in SECTIONS]
    for target in manifest.get("target", {}).values():
        tables += [target.get(section, {}) for section in SECTIONS]
    for table in tables:
        for key, spec in table.items():
            package = spec.get("package", key) if isinstance(spec, dict) else key
            names.add(package)
    return names


def crates() -> list[str]:
    listed = [(manifest["package"]["name"], manifest) for _, manifest in published()]
    order = {name: index for index, (name, _) in enumerate(listed)}
    needs = {name: dependency_names(manifest) & set(order) - {name} for name, manifest in listed}
    done: list[str] = []
    while len(done) < len(listed):
        ready = sorted((n for n in order if n not in done and needs[n] <= set(done)), key=order.get)
        if not ready:
            cycle = sorted(n for n in order if n not in done)
            raise SystemExit(f"the published crates name one another in a cycle: {cycle}")
        done.append(ready[0])
    return done


def version() -> str:
    workspace = toml(ROOT / "Cargo.toml")["workspace"]
    expected = workspace["package"]["version"]
    found: dict[str, str] = {"Cargo.toml [workspace.package]": expected}
    names = {manifest["package"]["name"] for _, manifest in members()}
    for key, spec in workspace.get("dependencies", {}).items():
        if key in names and isinstance(spec, dict) and "version" in spec:
            found[f"Cargo.toml [workspace.dependencies] {key}"] = spec["version"].removeprefix("=")
            if not spec["version"].startswith("="):
                raise SystemExit(f"Cargo.toml pins {key} at {spec['version']!r}; a workspace crate is pinned exactly, =<version>")
    for member, manifest in published():
        package = manifest["package"]
        spelled = package.get("version")
        if spelled == {"workspace": True}:
            continue
        found[f"{member}/Cargo.toml [package]"] = str(spelled)
    for path in ("python/pyproject.toml", "python/market/pyproject.toml"):
        if (ROOT / path).exists():
            project = toml(ROOT / path)["project"]
            found[f"{path} [project]"] = project["version"]
            for requirement in project.get("dependencies", []):
                if requirement.replace(" ", "").startswith("yggdryl=="):
                    found[f"{path} requires yggdryl"] = requirement.replace(" ", "").removeprefix("yggdryl==")
    for path in ("node/package.json", "node/market/package.json"):
        if (ROOT / path).exists():
            manifest = json.loads((ROOT / path).read_text(encoding="utf-8"))
            found[path] = manifest["version"]
            peer = manifest.get("peerDependencies", {}).get("yggdryl")
            if peer is not None:
                found[f"{path} peer yggdryl"] = peer
    for path in ("node/package-lock.json", "node/market/package-lock.json"):
        if (ROOT / path).exists():
            lock = json.loads((ROOT / path).read_text(encoding="utf-8"))
            found[path] = lock["version"]
            for key, entry in lock.get("packages", {}).items():
                if key in ("", "..") and "version" in entry:
                    found[f"{path} packages[{key!r}]"] = entry["version"]
    if len(set(found.values())) != 1:
        raise SystemExit(f"expected one version everywhere, got {json.dumps(found, indent=2)}")
    return expected


def main(argv: list[str]) -> int:
    if argv == ["version"]:
        print(version())
    elif argv == ["crates"]:
        print(" ".join(crates()))
    elif argv == ["pypi"]:
        print(" ".join(PYPI))
    elif argv == ["npm"]:
        print(" ".join(NPM))
    else:
        raise SystemExit(__doc__)
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
