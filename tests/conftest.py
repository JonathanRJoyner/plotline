"""Pytest options and fixtures shared by the test modules."""

import pytest


def pytest_addoption(parser):
    parser.addoption(
        "--update-golden",
        action="store_true",
        help="Rewrite the files under tests/golden instead of comparing against them.",
    )


@pytest.fixture
def update_golden(request):
    return request.config.getoption("--update-golden")
