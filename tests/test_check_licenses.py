import sys
from types import SimpleNamespace

sys.path.insert(0, "scripts")
import check_licenses as cl  # noqa: E402


def test_allows_permissive_texts():
    for t in [
        "MIT",
        "License :: OSI Approved :: BSD License",
        "Apache-2.0",
        "MIT-CMU",
        "Historical Permission Notice and Disclaimer (HPND)",
        "PSF-2.0",
        "ISC",
    ]:
        assert cl.is_allowed(t), t


def test_denies_copyleft_and_empty():
    for t in ["GPL-3.0-only", "LGPL-2.1", "MIT OR LGPL-3.0", "MPL-2.0", ""]:
        assert not cl.is_allowed(t), t


def test_closure_is_runtime_only():
    names = cl.runtime_closure()
    assert "numpy" in names and "click" in names
    assert "pydantic" not in names and "scipy" not in names  # dev group only
    assert "pytest" not in names and "numba" not in names and "plotline" not in names
    with_mcp = cl.runtime_closure(extras=("mcp",))
    assert "mcp" in with_mcp and "pydantic" in with_mcp
    assert "cryptography" in with_mcp  # reached through the requested pyjwt[crypto] extra


def test_environment_passes():
    assert cl.main() == 0


def test_tokens_match_whole_words_only():
    assert cl.is_allowed(
        "THIS SOFTWARE IS PROVIDED WITHOUT IMPLIED WARRANTIES. License :: OSI Approved :: BSD License"
    )
    assert not cl.is_allowed("PERMITTED USE ONLY")


def test_glued_copyleft_spellings_are_denied():
    for t in [
        "MIT OR LGPLv3",
        "MIT/GPLv2",
        "License :: OSI Approved :: GNU General Public License v3 (GPLv3)",
    ]:
        assert not cl.is_allowed(t), t
    assert cl.is_allowed("THIS SOFTWARE IS PROVIDED WITHOUT IMPLIED WARRANTIES. BSD-3-Clause")


def test_main_fails_on_a_denied_licence(monkeypatch, capsys):
    monkeypatch.setattr(cl, "runtime_closure", lambda **kw: ["demo"])
    monkeypatch.setattr(cl, "distribution", lambda name: None)
    monkeypatch.setattr(cl, "license_text", lambda dist: "GPL-3.0-only")
    assert cl.main() == 1
    assert "demo" in capsys.readouterr().out


def test_closure_reaches_transitive_dependencies(monkeypatch):
    graph = {"plotline": ["a>=1"], "a": ["b; python_version >= '3'"], "b": []}
    monkeypatch.setattr(cl, "distribution", lambda name: SimpleNamespace(requires=graph[name]))
    assert cl.runtime_closure() == ["a", "b"]


def _fake_graph(monkeypatch, graph):
    monkeypatch.setattr(cl, "distribution", lambda name: SimpleNamespace(requires=graph[name]))


def test_closure_follows_only_the_roots_chosen_extras(monkeypatch):
    # "x" and "y" are absent from the graph: following them would raise KeyError.
    _fake_graph(
        monkeypatch,
        {
            "plotline": ["a>=1", 'm>=2; extra == "mcp"'],
            "a": ['x; extra == "cli"', 'y; extra == "mcp"'],
            "m": [],
        },
    )
    assert cl.runtime_closure() == ["a"]
    assert cl.runtime_closure(extras=("mcp",)) == ["a", "m"]


def test_closure_follows_extras_a_requirement_asks_for(monkeypatch):
    _fake_graph(
        monkeypatch,
        {"plotline": ["a[c]>=1"], "a": ['z; extra == "c"', 'w; extra == "d"'], "z": []},
    )
    assert cl.runtime_closure() == ["a", "z"]


def test_closure_follows_extras_asked_for_on_a_later_visit(monkeypatch):
    _fake_graph(
        monkeypatch,
        {"plotline": ["a", "b"], "b": ["a[c]"], "a": ['z; extra == "c"'], "z": []},
    )
    assert cl.runtime_closure() == ["a", "b", "z"]


def test_closure_reads_every_extra_a_marker_names(monkeypatch):
    _fake_graph(
        monkeypatch,
        {
            "plotline": [
                'm; extra == "x" or extra == "mcp"',
                'n; extra=="mcp"',
                'o; python_version >= "3" and extra == "x"',
            ],
            "m": [],
            "n": [],
        },
    )
    assert cl.runtime_closure(extras=("mcp",)) == ["m", "n"]


def test_matching_is_case_sensitive_and_bounded_on_both_sides():
    assert not cl.is_allowed("mit")
    assert not cl.is_allowed("SUBMIT")
    assert not cl.is_allowed("MITIGATION")
