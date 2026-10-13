"""Fail when a runtime dependency of plotline lacks a permissive licence.

Standard library only. The script walks the installed ``Requires-Dist`` graph from the project,
follows the project's shipped extras (the ``mcp`` one) and any extra a requirement asks for, as in
``pyjwt[crypto]``, skips every other optional extra, and judges each distribution by the licence
text in its metadata: the ``License-Expression`` field (PEP 639), the ``License`` field and the
``License ::`` classifiers of the core metadata specification.

Matching is deliberately crude: a licence passes when its text contains an ALLOWED token as a
whole word and no DENIED token at the start of a word, case-sensitive. Anything unclear fails. A
person then checks the package and, when its real licence is permissive, records the result in
OVERRIDES.
"""

import re
import sys
import textwrap
from collections import deque
from importlib.metadata import Distribution, PackageNotFoundError, distribution

ALLOWED: tuple[str, ...] = (
    "MIT",
    "BSD",
    "Apache",
    "ISC",
    "HPND",
    "Historical Permission",
    "PSF",
    "Python Software Foundation",
    "Python-2.0",
)
DENIED: tuple[str, ...] = ("GPL", "LGPL", "AGPL", "MPL", "EPL", "CDDL", "SSPL")

# Normalised package name -> (licence, URL where the licence was checked). For packages whose
# metadata trips a DENIED token for a reason that does not change their own licence.
OVERRIDES: dict[str, tuple[str, str]] = {}


def _normalise(name: str) -> str:
    return re.sub(r"[-_.]+", "-", name).lower()


def license_text(dist: Distribution) -> str:
    """Join the licence fields of a distribution's metadata into one string."""
    meta = dist.metadata
    parts = [meta.get("License-Expression"), meta.get("License")]
    parts += [c for c in meta.get_all("Classifier", []) if c.startswith("License ::")]
    return "\n".join(p for p in parts if p)


def is_allowed(text: str) -> bool:
    """True when an ALLOWED token is a whole word and no DENIED token starts a word.

    Case-sensitive: "MIT-CMU" has "MIT" and "GPLv3" has "GPL", but "IMPLIED" has no "MPL".
    """
    allowed = any(re.search(r"\b" + re.escape(t) + r"\b", text) for t in ALLOWED)
    return allowed and not any(re.search(r"\b" + re.escape(t), text) for t in DENIED)


def runtime_closure(root: str = "plotline", extras: tuple[str, ...] = ()) -> list[str]:
    """Names of the installed distributions that ``root`` needs at runtime, breadth first.

    A requirement behind ``extra == "<x>"`` is followed only when ``<x>`` was requested for the
    distribution that lists it: through ``extras`` for ``root``, through a bracket such as
    ``pyjwt[crypto]`` for the others.
    """
    root_name = _normalise(root)
    followed: dict[str, set[str]] = {}  # name -> the extras already followed for it
    queue = deque([(distribution(root), {_normalise(e) for e in extras})])  # a missing root fails
    while queue:
        current, requested = queue.popleft()
        for req in current.requires or []:
            spec, _, marker = req.partition(";")
            named = {_normalise(e) for e in re.findall(r"extra\s*==\s*[\"']([^\"']+)[\"']", marker)}
            if named and requested.isdisjoint(named):
                continue
            name = _normalise(re.split(r"[^A-Za-z0-9._-]", spec.strip(), maxsplit=1)[0])
            bracket = re.search(r"\[([^\]]*)\]", spec)
            asked = (
                {_normalise(e) for e in bracket[1].split(",") if e.strip()} if bracket else set()
            )
            if name == root_name or (name in followed and asked <= followed[name]):
                continue
            try:
                dist = distribution(name)
            except PackageNotFoundError:
                continue  # a dependency for another platform, not installed here
            followed.setdefault(name, set()).update(asked)
            queue.append((dist, set(followed[name])))
    return sorted(followed)


def main() -> int:
    offenders: list[str] = []
    for name in runtime_closure(extras=("mcp",)):
        text = license_text(distribution(name))
        override = OVERRIDES.get(name)
        ok = override is not None or is_allowed(text)
        shown = f"{override[0]} (override)" if override else text
        label = textwrap.shorten(shown, width=70, placeholder="...") or "(none)"
        print(f"{name}  {label}  {'OK' if ok else 'DENIED'}")
        if not ok:
            offenders.append(name)
    if offenders:
        print("No permissive licence found for: " + ", ".join(offenders))
        print("Check the real licence of each, then record it in OVERRIDES.")
    return 1 if offenders else 0


if __name__ == "__main__":
    sys.exit(main())
