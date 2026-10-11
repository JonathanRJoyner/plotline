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
