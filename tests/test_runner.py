"""Tests for the runner: content-hash keys, cache hits and misses, cancellation and the result."""

import numpy as np
import pytest
from doubles import Spy, SpySettings, collect

from plotline.core.cache import StageCache
from plotline.core.pipeline import Style, root_key, run_style, stage_key
from plotline.core.stage import Cancelled, CancelToken, RunContext
from plotline.core.types import Canvas, Layer, Pen, ToneMap

ROOTS = {
    "image": np.zeros((4, 4, 3), np.float32),
    "canvas": Canvas(100, 100),
    "pens": (Pen("p", 0.3, "#000000"),),
}
PATHS = [np.array([[1.0, 1.0], [2.0, 2.0]])]


def make(frames_b=1):
    a = Spy("a", ("image",), "points", value=np.zeros((3, 2)))
    b = Spy("b", ("points",), "paths", value=PATHS, frames=frames_b)
    return Style("t", "T", "d", (a, b)), a, b


def run(style, settings, cache, roots=ROOTS, token=None):
    return collect(run_style(style, settings, roots, cache, RunContext(token or CancelToken())))


def test_runs_every_stage_and_wraps_paths_in_a_layer():
    style, a, b = make()
    frames, result = run(style, style.defaults(), StageCache())
    assert (a.runs, b.runs) == (1, 1) and [f.stage for f in frames] == ["a", "b"]
    assert result.layers[0].pen == ROOTS["pens"][0] and result.layers[0].paths == PATHS
    assert result.canvas == ROOTS["canvas"] and result.tone is None


def test_second_run_is_served_from_cache():
    style, a, b = make()
    cache = StageCache()
    run(style, style.defaults(), cache)
    frames, _ = run(style, style.defaults(), cache)
    assert (a.runs, b.runs) == (1, 1) and [f.label for f in frames] == ["cached", "cached"]


def test_late_change_reruns_late_stages_only():
    style, a, b = make()
    cache = StageCache()
    run(style, style.defaults(), cache)
    run(style, {**style.defaults(), "b": SpySettings(knob=1)}, cache)
    assert (a.runs, b.runs) == (1, 2)


def test_early_change_and_root_change_rerun_everything():
    style, a, b = make()
    cache = StageCache()
    run(style, style.defaults(), cache)
    run(style, {**style.defaults(), "a": SpySettings(knob=1)}, cache)
    run(style, style.defaults(), cache, roots={**ROOTS, "image": np.ones((4, 4, 3), np.float32)})
    assert (a.runs, b.runs) == (3, 3)


def test_equivalent_stage_in_another_style_hits_the_cache():
    style, a, b = make()
    cache = StageCache()
    run(style, style.defaults(), cache)
    a2 = Spy("a", ("image",), "points", value=np.zeros((3, 2)))
    other = Style("u", "U", "d", (a2, Spy("b2", ("points",), "paths", value=PATHS)))
    run(other, other.defaults(), cache)
    assert a2.runs == 0


def test_cancel_discards_partial_output_and_keeps_earlier_stages():
    style, a, b = make(frames_b=5)
    cache = StageCache()
    token = CancelToken()
    gen = run_style(style, style.defaults(), ROOTS, cache, RunContext(token))
    seen = 0
    with pytest.raises(Cancelled):
        for frame in gen:
            if frame.stage == "b":
                seen += 1
                if seen == 2:
                    token.set()
    assert len(cache) == 1 and seen == 2


def test_layers_provider_and_tone_are_used_directly():
    tone = ToneMap(np.zeros((2, 2), np.float32), 1.0, (0.0, 0.0))
    layers = [Layer(Pen("q", 0.5, "#ff0000"), PATHS)]
    style = Style(
        "t",
        "T",
        "d",
        (Spy("t", ("image",), "tone", value=tone), Spy("l", ("tone",), "layers", value=layers)),
    )
    _, result = run(style, style.defaults(), StageCache())
    assert result.layers is layers and result.tone is tone


def test_missing_root_raises():
    style, _, _ = make()
    with pytest.raises(KeyError):
        run(style, style.defaults(), StageCache(), roots={"canvas": ROOTS["canvas"]})


def test_missing_stage_settings_raise():
    style, _, _ = make()
    with pytest.raises(KeyError):
        run(style, {"a": SpySettings()}, StageCache())


def test_stored_none_artifact_counts_as_a_hit():
    a = Spy("a", ("image",), "points")
    style = Style("t", "T", "d", (a, Spy("b", ("points",), "paths", value=PATHS)))
    cache = StageCache()
    run(style, style.defaults(), cache)
    frames, _ = run(style, style.defaults(), cache)
    assert a.runs == 1 and (cache.hits, cache.misses) == (2, 2)
    assert [(f.stage, f.progress, f.label) for f in frames] == [
        ("a", 1.0, "cached"),
        ("b", 1.0, "cached"),
    ]


def test_cancel_is_honoured_after_a_cached_frame():
    style, a, b = make()
    cache = StageCache()
    run(style, style.defaults(), cache)
    token = CancelToken()
    gen = run_style(style, style.defaults(), ROOTS, cache, RunContext(token))
    next(gen)
    token.set()
    with pytest.raises(Cancelled):
        next(gen)


def test_tone_comes_from_the_last_stage_that_provides_it():
    first = ToneMap(np.zeros((2, 2), np.float32), 1.0, (0.0, 0.0))
    last = ToneMap(np.ones((2, 2), np.float32), 1.0, (0.0, 0.0))
    stages = (
        Spy("t1", ("image",), "tone", value=first),
        Spy("t2", ("tone",), "tone", value=last),
        Spy("p", ("tone",), "paths", value=PATHS),
    )
    style = Style("t", "T", "d", stages)
    _, result = run(style, style.defaults(), StageCache())
    assert result.tone is last


def test_a_stage_that_provides_a_root_kind_feeds_later_stages():
    cropped = np.ones((2, 2, 3), np.float32)
    stages = (
        Spy("crop", ("image",), "image", value=cropped),
        Spy("draw", ("image",), "paths", value=lambda inputs: [inputs["image"]]),
    )
    style = Style("t", "T", "d", stages)
    _, result = run(style, style.defaults(), StageCache())
    assert result.layers[0].paths[0] is cropped


def test_root_keys_follow_content():
    image = np.arange(48, dtype=np.float32).reshape(4, 4, 3)
    assert root_key("image", image) == root_key("image", image.copy())
    assert root_key("image", image) == root_key("image", np.asfortranarray(image))
    assert root_key("image", image) != root_key("image", image.reshape(2, 8, 3))
    assert root_key("image", image) != root_key("image", image.view(np.int32))
    assert root_key("canvas", Canvas(100, 100)) == root_key("canvas", Canvas(100, 100))
    assert root_key("canvas", Canvas(100, 100)) != root_key("canvas", Canvas(100, 100, 5))
    p, q = Pen("p", 0.3, "#000000"), Pen("q", 0.5, "#ff0000")
    assert root_key("pens", (p, q)) != root_key("pens", (q, p))
    assert root_key("pens", (p,)) != root_key("pens", (q,))
    with pytest.raises(ValueError):
        root_key("tone", image)


class OtherSpy(Spy):
    """The same stage under another class, which the stage key tells apart."""


def spy_key(cls=Spy, name="a", version="1", knob=0, image="k"):
    spy = cls(name, ("image",), "points")
    spy.version = version
    return stage_key(spy, SpySettings(knob=knob), {"image": image})


def test_stage_key_follows_class_name_version_settings_and_inputs():
    keys = {
        spy_key(),
        spy_key(cls=OtherSpy),
        spy_key(name="b"),
        spy_key(version="2"),
        spy_key(knob=1),
        spy_key(image="other"),
    }
    assert len(keys) == 6 and spy_key() == spy_key()
