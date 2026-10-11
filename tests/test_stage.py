"""Tests for the stage contract: frames, cancellation, the run context and the abstract base."""

import pytest
from doubles import collect

from plotline.core.settings import Settings
from plotline.core.stage import Cancelled, CancelToken, Frame, RunContext, Stage


class Two(Stage):
    name, label, requires, provides, settings_model = "two", "Two", (), "n", Settings

    def run(self, inputs, settings, ctx):
        yield Frame(0.5, "half")
        yield Frame(1.0, "done")
        return 2


def test_stage_yields_frames_then_returns_artifact():
    frames, value = collect(Two().run({}, Settings(), RunContext(CancelToken())))
    assert [f.label for f in frames] == ["half", "done"] and value == 2
    assert frames[0].points is None and frames[0].paths is None and frames[0].stage == ""


def test_check_raises_only_when_cancelled():
    token = CancelToken()
    ctx = RunContext(token)
    ctx.check()
    token.set()
    assert token.is_set()
    with pytest.raises(Cancelled):
        ctx.check()


def test_rng_is_seeded():
    ctx = RunContext(CancelToken())
    assert ctx.rng(7).random() == ctx.rng(7).random()
    assert ctx.rng(7).random() != ctx.rng(8).random()


def test_stage_is_abstract():
    with pytest.raises(TypeError):
        Stage()
