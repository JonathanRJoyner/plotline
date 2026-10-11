import sys

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
    assert "numpy" in names and "pydantic" in names and "click" in names
    assert "pytest" not in names and "numba" not in names and "plotline" not in names


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
    monkeypatch.setattr(cl, "runtime_closure", lambda: ["demo"])
    monkeypatch.setattr(cl, "distribution", lambda name: None)
    monkeypatch.setattr(cl, "license_text", lambda dist: "GPL-3.0-only")
    assert cl.main() == 1
    assert "demo" in capsys.readouterr().out


def test_closure_reaches_transitive_dependencies():
    assert "pydantic-core" in cl.runtime_closure()


def test_matching_is_case_sensitive_and_bounded_on_both_sides():
    assert not cl.is_allowed("mit")
    assert not cl.is_allowed("SUBMIT")
    assert not cl.is_allowed("MITIGATION")
