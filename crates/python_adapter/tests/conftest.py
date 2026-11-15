"""Check the wire contract against responses from the installed extension."""

import json
from collections import defaultdict
from functools import wraps
from importlib.resources import files
from pathlib import Path

import pytest
from jsonschema import Draft202012Validator, ValidationError

import memory_engine


NATIVE_RESULTS = {None, "BuildInfo", "int", "str"}
# These fields can contain arbitrary host/provider data. Their contents are
# outside the schema of the enclosing response.
OPEN_OBJECTS = {"domain_state", "inputs", "payload", "prompt_inputs"}
RECORDED_RESPONSES = pytest.StashKey()


def pytest_addoption(parser):
    parser.addoption("--require-api-contract-coverage", action="store_true")
    parser.addoption("--record-api-response-schemas", action="store_true")


@pytest.fixture(scope="session")
def check_api_response(pytestconfig):
    package = files(memory_engine)
    contracts = json.loads(package.joinpath("api_contracts.json").read_text(encoding="utf-8"))
    recording = pytestconfig.getoption("--record-api-response-schemas")
    require_coverage = pytestconfig.getoption("--require-api-contract-coverage")
    validators = {}
    samples = defaultdict(list)
    observed_calls = set()
    if not recording:
        bundle = json.loads(package.joinpath("response_schemas.json").read_text(encoding="utf-8"))
        validators = {
            name: Draft202012Validator(schema) for name, schema in bundle["$defs"].items()
        }

    def check(method, result):
        result_name = contracts[method]["result"]
        value = result if result_name in NATIVE_RESULTS else json.loads(result)
        if result_name == "int":
            assert type(value) is int, method
        if result_name is not None and not recording:
            try:
                validators[result_name].validate(value)
            except ValidationError as error:
                pytest.fail(
                    f"{method} returned an invalid {result_name} at {error.json_path}: {error.message}"
                )
        if result_name is not None and (recording or require_coverage):
            samples[result_name].append(value)
        observed_calls.add(method)
        return result

    yield check

    if recording or require_coverage:
        assert observed_calls == contracts.keys(), (
            "Missing successful API responses: " + ", ".join(sorted(contracts.keys() - observed_calls))
        )
    if recording:
        pytestconfig.stash[RECORDED_RESPONSES] = samples
    elif require_coverage:
        expected = bundle["$defs"]
        actual = response_schemas(samples)
        changed = sorted(
            name for name in expected.keys() | actual.keys()
            if expected.get(name) != actual.get(name)
        )
        assert not changed, "Response shapes differ from the installed schemas: " + ", ".join(changed)


@pytest.fixture
def engine_factory(check_api_response):
    def create(path, **kwargs):
        native = memory_engine.MemoryEngine(str(path), **kwargs)
        check_api_response("MemoryEngine.__init__", None)

        class CheckedEngine:
            def __getattr__(self, name):
                method = getattr(native, name)
                if name.startswith("_") or not callable(method):
                    return method

                @wraps(method)
                def call(*args, **options):
                    return check_api_response(f"MemoryEngine.{name}", method(*args, **options))

                return call

        return CheckedEngine()

    return create


def pytest_sessionfinish(session, exitstatus):
    if exitstatus != pytest.ExitCode.OK or RECORDED_RESPONSES not in session.config.stash:
        return
    schemas = response_schemas(session.config.stash[RECORDED_RESPONSES])
    bundle = {"$schema": "https://json-schema.org/draft/2020-12/schema", "$defs": schemas}
    Draft202012Validator.check_schema(bundle)
    destination = Path(__file__).parents[1] / "python" / "memory_engine" / "response_schemas.json"
    destination.write_text(json.dumps(bundle, indent=2) + "\n", encoding="utf-8", newline="\n")


def response_schemas(samples):
    schemas = {name: observed_schema(values) for name, values in sorted(samples.items())}
    # Either field can independently be unknown in a build from git archive.
    schemas["BuildInfo"]["properties"].update(
        commit={"type": ["string", "null"], "pattern": r"^(?:[0-9a-f]{40}|[0-9a-f]{64})$"},
        dirty={"type": ["boolean", "null"]},
    )
    return schemas


def observed_schema(values, depth=0, field=None):
    """Record structure and schema versions without copying fixture content."""
    groups = defaultdict(list)
    for value in values:
        kind = {
            type(None): "null", bool: "boolean", int: "integer", float: "number",
            str: "string", list: "array", dict: "object",
        }[type(value)]
        groups[kind].append(value)
    if "integer" in groups and "number" in groups:
        groups["number"].extend(groups.pop("integer"))

    variants = []
    for kind, examples in sorted(groups.items()):
        schema = {"type": kind}
        if kind == "string" and field == "schema_version":
            schema["enum"] = sorted(set(examples))
        elif kind == "object" and depth < 3 and field not in OPEN_OBJECTS:
            keys = sorted(set().union(*(value.keys() for value in examples)))
            schema["properties"] = {
                key: observed_schema([value[key] for value in examples if key in value], depth + 1, key)
                for key in keys
            }
            schema["required"] = [key for key in keys if all(key in value for value in examples)]
            schema["additionalProperties"] = False
        elif kind == "array":
            items = [item for value in examples for item in value]
            if items:
                schema["items"] = observed_schema(items, depth, field)
        variants.append(schema)
    return variants[0] if len(variants) == 1 else {"anyOf": variants}
