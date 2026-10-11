"""Fail when a runtime dependency of plotline lacks a permissive licence.

Standard library only. The script walks the installed ``Requires-Dist`` graph from the project,
skips optional extras, and judges each distribution by the licence text in its metadata: the
``License-Expression`` field (PEP 639), the ``License`` field and the ``License ::``
classifiers of the core metadata specification.

Matching is deliberately crude: a licence passes when its text contains an ALLOWED token and no
DENIED token, each matched as a whole word and case-sensitive. Anything unclear fails. A person
then checks the package and, when its real licence is permissive, records the result in OVERRIDES.
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
OVERRIDES: dict[str, tuple[str, str]] = {
    # SciPy's own licence is BSD-3-Clause. Its wheel metadata also carries the notice for a
    # bundled compiler runtime library under GPL-3.0-or-later with a runtime library exception.
    "scipy": ("BSD-3-Clause", "https://raw.githubusercontent.com/scipy/scipy/v1.17.1/LICENSE.txt"),
}


def _normalise(name: str) -> str:
    return re.sub(r"[-_.]+", "-", name).lower()


def license_text(dist: Distribution) -> str:
    """Join the licence fields of a distribution's metadata into one string."""
    meta = dist.metadata
    parts = [meta.get("License-Expression"), meta.get("License")]
    parts += [c for c in meta.get_all("Classifier", []) if c.startswith("License ::")]
    return "\n".join(p for p in parts if p)


def _has_token(token: str, text: str) -> bool:
    # "-" is a word boundary, so "MIT-CMU" holds "MIT"; "IMPLIED" does not hold "MPL".
    return re.search(r"\b" + re.escape(token) + r"\b", text) is not None


def is_allowed(text: str) -> bool:
    allowed = any(_has_token(token, text) for token in ALLOWED)
    return allowed and not any(_has_token(token, text) for token in DENIED)


def runtime_closure(root: str = "plotline") -> list[str]:
    """Names of the installed distributions that ``root`` needs at runtime, breadth first."""
    root_name = _normalise(root)
    names: set[str] = set()
    queue = deque([distribution(root)])  # a missing root fails loudly instead of passing
    while queue:
        for req in queue.popleft().requires or []:
            spec, _, marker = req.partition(";")
            if "extra ==" in marker:
                continue
            name = _normalise(re.split(r"[^A-Za-z0-9._-]", spec.strip(), maxsplit=1)[0])
            if name == root_name or name in names:
                continue
            try:
                dist = distribution(name)
            except PackageNotFoundError:
                continue  # a dependency for another platform, not installed here
            names.add(name)
            queue.append(dist)
    return sorted(names)


def main() -> int:
    offenders: list[str] = []
    for name in runtime_closure():
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
