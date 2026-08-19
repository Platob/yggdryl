"""Jinja-style `{{ }}` placeholders: a closed grammar, opt-in, and env-sealed."""

from __future__ import annotations

import json as stdlib_json
import os
import pathlib

import pytest

from yggdryl import json, toml, yaml

# The same document, written the way each format spells it. YAML *requires* the
# quotes: a bare `{{ X }}` is a flow mapping, not a scalar.
DOCUMENTS = [
    (json, lambda quoted: f"{{\"value\": {quoted}}}"),
    (yaml, lambda quoted: f"value: {quoted}\n"),
    (toml, lambda quoted: f"value = {quoted}\n"),
]


def resolved(scalar: str, **filling: object) -> list[object]:
    quoted = stdlib_json.dumps(scalar)
    return [
        module.loads(document(quoted), **filling)["value"]
        for module, document in DOCUMENTS
    ]


def test_a_whole_scalar_placeholder_adopts_the_resolved_value_s_type() -> None:
    variables = {"PORT": 8080, "DEBUG": True, "HOSTS": ["a", "b"], "NOTHING": None}
    assert resolved("{{ PORT }}", placeholders=variables) == [8080] * 3
    assert resolved("{{ DEBUG }}", placeholders=variables) == [True] * 3
    assert resolved("{{ HOSTS }}", placeholders=variables) == [["a", "b"]] * 3
    assert resolved("{{ NOTHING }}", placeholders=variables) == [None] * 3


def test_an_embedded_placeholder_is_textual_and_stays_a_string() -> None:
    variables = {"ROOT": "/var/log", "PORT": 8080}
    assert resolved("{{ ROOT }}/app", placeholders=variables) == ["/var/log/app"] * 3
    assert resolved("h:{{ PORT }}/x", placeholders=variables) == ["h:8080/x"] * 3

    # A container has no text form inside a larger string.
    with pytest.raises(ValueError, match="resolve to a scalar"):
        json.loads('{"a": "x{{ HOSTS }}"}', placeholders={"HOSTS": ["a"]})


def test_a_missing_variable_names_itself_rather_than_resolving_to_nothing() -> None:
    with pytest.raises(ValueError) as failure:
        json.loads('{"a": {"b": "{{ MISSING }}"}}', placeholders={})
    message = str(failure.value)
    assert "MISSING" in message
    assert "$.a.b" in message
    assert "at byte 0" in message


def test_a_default_makes_a_variable_optional_and_carries_its_own_type() -> None:
    assert resolved('{{ PORT | default(8080) }}', placeholders={}) == [8080] * 3
    assert resolved('{{ R | default("/tmp") }}', placeholders={}) == ["/tmp"] * 3
    assert resolved('{{ ON | default(true) }}', placeholders={}) == [True] * 3
    # A supplied value wins over the default.
    assert resolved('{{ P | default(1) }}', placeholders={"P": 2}) == [2] * 3

    # `default` is the only filter there is, and anything else says so.
    with pytest.raises(ValueError, match="default\\(LITERAL\\)"):
        json.loads('{"a": "{{ R | upper }}"}', placeholders={"R": "x"})


def test_a_doubled_opener_is_a_literal_one() -> None:
    assert resolved("{{{{ NAME }}", placeholders={}) == ["{{ NAME }}"] * 3
    with pytest.raises(ValueError, match="unterminated"):
        json.loads('{"a": "{{ NAME"}', placeholders={})


def test_substitution_is_off_unless_asked_for() -> None:
    # No `placeholders`, no `environment`: the braces are ordinary text, and
    # not even a missing variable is looked for.
    assert json.loads('{"a": "{{ MISSING }}"}')["a"] == "{{ MISSING }}"
    assert yaml.loads('a: "{{ MISSING }}"\n')["a"] == "{{ MISSING }}"


def test_the_environment_is_a_second_switch_and_the_mapping_wins() -> None:
    name = "YGGDRYL_PLACEHOLDER_PYTEST_VALUE"
    os.environ[name] = "from-environment"
    try:
        scalar = f"{{{{ {name} }}}}"
        # Set, and still not resolved: the environment was not consulted.
        with pytest.raises(ValueError, match="not consulted"):
            resolved(scalar, placeholders={})

        assert resolved(scalar, environment=True) == ["from-environment"] * 3
        overridden = resolved(
            scalar, placeholders={name: "from-mapping"}, environment=True
        )
        assert overridden == ["from-mapping"] * 3
    finally:
        del os.environ[name]


def test_a_document_without_placeholders_parses_identically_either_way(
    tmp_path: pathlib.Path,
) -> None:
    document = '{"a": "plain", "b": [1, 2], "c": {"d": null}}'
    assert json.loads(document, placeholders={"X": 1}) == json.loads(document)

    # And through every source shape the loader accepts.
    target = tmp_path / "config.json"
    target.write_text('{"a": "{{ NAME }}"}')
    expected = {"a": "app"}
    filling = {"placeholders": {"NAME": "app"}}
    assert json.loads(target, **filling) == expected
    assert json.loads(target.read_bytes(), **filling) == expected
    with target.open("rb") as stream:
        assert json.loads(stream, **filling) == expected


def test_an_unquoted_yaml_placeholder_is_what_yaml_says_it_is() -> None:
    filling = {"placeholders": {"PORT": 8080}}
    assert yaml.loads('port: "{{ PORT }}"\n', **filling)["port"] == 8080

    # Unquoted, YAML read a flow mapping before anything here ran, and nothing
    # here rewrites the document's shape to pretend otherwise.
    bare = yaml.loads("port: {{ PORT }}\n", **filling)["port"]
    assert isinstance(bare, dict), bare


def test_dumping_never_reintroduces_a_placeholder(tmp_path: pathlib.Path) -> None:
    value = json.loads('{"path": "{{ ROOT }}/x"}', placeholders={"ROOT": "/srv"})
    assert json.dumps(value) == b'{"path":"/srv/x"}'
