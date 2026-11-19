import ast
import inspect
import json
from importlib.resources import files

import pytest
from jsonschema import Draft202012Validator, ValidationError

import memory_engine


def stub_signature(node):
    args = node.args
    positional = args.posonlyargs + args.args
    defaults = [inspect.Parameter.empty] * (len(positional) - len(args.defaults))
    defaults.extend(ast.literal_eval(value) for value in args.defaults)
    params = [
        inspect.Parameter(
            arg.arg,
            inspect.Parameter.POSITIONAL_ONLY if index < len(args.posonlyargs)
            else inspect.Parameter.POSITIONAL_OR_KEYWORD,
            default=default,
        )
        for index, (arg, default) in enumerate(zip(positional, defaults))
    ]
    params.extend(
        inspect.Parameter(
            arg.arg, inspect.Parameter.KEYWORD_ONLY,
            default=ast.literal_eval(default) if default is not None else inspect.Parameter.empty,
        )
        for arg, default in zip(args.kwonlyargs, args.kw_defaults)
    )
    assert args.vararg is None and args.kwarg is None
    return inspect.Signature(params)


def test_installed_contract_matches_native_methods_and_stub(pytestconfig):
    package = files(memory_engine)
    assert package.joinpath("py.typed").is_file()
    stub = ast.parse(package.joinpath("__init__.pyi").read_text(encoding="utf-8"))
    engine = next(node for node in stub.body if isinstance(node, ast.ClassDef) and node.name == "MemoryEngine")
    methods = {
        f"MemoryEngine.{node.name}": node for node in engine.body if isinstance(node, ast.FunctionDef)
    }
    methods.update({node.name: node for node in stub.body if isinstance(node, ast.FunctionDef)})

    contracts = json.loads(package.joinpath("api_contracts.json").read_text(encoding="utf-8"))
    runtime_methods = {
        f"MemoryEngine.{name}"
        for name, member in inspect.getmembers(memory_engine.MemoryEngine, inspect.ismethoddescriptor)
        if not name.startswith("_")
    }
    runtime_methods.add("MemoryEngine.__init__")
    runtime_methods.update(name for name in memory_engine.__all__ if inspect.isbuiltin(getattr(memory_engine, name)))
    assert contracts.keys() == methods.keys() == runtime_methods

    for name, entry in contracts.items():
        stub_method = methods[name]
        short_name = name.rsplit(".", 1)[-1]
        expected = stub_signature(stub_method)
        if short_name == "__init__":
            native = memory_engine.MemoryEngine
            expected = expected.replace(parameters=list(expected.parameters.values())[1:])
        else:
            owner = memory_engine.MemoryEngine if name.startswith("MemoryEngine.") else memory_engine
            native = getattr(owner, short_name)
        assert inspect.signature(native) == expected, name
        assert set(entry["request"]) == {param for param in expected.parameters if param.endswith("_json")}, name
        result_type = entry["result"]
        expected_result = {None: "None", "BuildInfo": "_BuildInfo", "int": "int"}.get(result_type, "str")
        assert ast.unparse(stub_method.returns) == expected_result, name

    if not pytestconfig.getoption("--record-api-response-schemas"):
        bundle = json.loads(package.joinpath("response_schemas.json").read_text(encoding="utf-8"))
        Draft202012Validator.check_schema(bundle)
        assert bundle["$defs"].keys() == {
            entry["result"] for entry in contracts.values() if entry["result"] is not None
        }


def test_response_schema_detects_renamed_field(tmp_path, pytestconfig):
    if pytestconfig.getoption("--record-api-response-schemas"):
        pytest.skip("This check uses the installed response schemas")
    bundle = json.loads(files(memory_engine).joinpath("response_schemas.json").read_text(encoding="utf-8"))
    adapter = memory_engine.MemoryEngine(str(tmp_path))
    actual = json.loads(adapter.read_session("new_session"))
    validator = Draft202012Validator(bundle["$defs"]["SessionRecord"])
    validator.validate(actual)
    actual["renamed_metadata"] = actual.pop("metadata")
    with pytest.raises(ValidationError, match="'metadata' is a required property"):
        validator.validate(actual)
