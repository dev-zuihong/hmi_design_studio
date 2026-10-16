"""One request per process. stdout is reserved for UTF-8 JSON-RPC 2.0.

This worker executes trusted project code; process limits are lifecycle controls,
not a security sandbox. The host starts the process suspended inside a Job.
"""
import os
import sys

# The macOS standalone package does not have a _pth file like Windows. Add
# only the hash-verified project dependency directory; never enable site or
# consult a machine-wide Python installation.
if sys.platform == "darwin":
    sys.path.insert(0, os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "dependencies"))

import ast
import ctypes
import importlib
import inspect
import io
import json
import math
import traceback
import types

MAX_MESSAGE_BYTES = 1024 * 1024


def isolate_output():
    protocol_fd = os.dup(1)
    os.set_inheritable(protocol_fd, False)
    os.dup2(2, 1)
    if os.name == "nt":
        import msvcrt

        kernel = ctypes.WinDLL("kernel32", use_last_error=True)
        kernel.SetStdHandle.argtypes = [ctypes.c_uint32, ctypes.c_void_p]
        kernel.SetStdHandle.restype = ctypes.c_int
        if not kernel.SetStdHandle(ctypes.c_uint32(-11), msvcrt.get_osfhandle(2)):
            raise ctypes.WinError(ctypes.get_last_error())
    sys.stdout = io.TextIOWrapper(os.fdopen(os.dup(2), "wb", buffering=0),
                                  encoding="utf-8", errors="replace", write_through=True)
    sys.stderr.reconfigure(encoding="utf-8", errors="replace", write_through=True)
    return os.fdopen(protocol_fd, "w", encoding="utf-8", newline="\n", buffering=1)


def validate_source(source):
    if not isinstance(source, str):
        raise TypeError("source must be a string")
    tree = ast.parse(source, filename="<python-node>", mode="exec")
    entries = [node for node in tree.body
               if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef))
               and node.name == "transform"]
    if len(entries) != 1 or not isinstance(entries[0], ast.FunctionDef):
        raise TypeError("Define exactly one synchronous def transform(msg)")
    entry = entries[0]
    args = entry.args
    if (entry.decorator_list or args.posonlyargs or len(args.args) != 1
            or args.args[0].arg != "msg" or args.vararg or args.kwarg
            or args.kwonlyargs or args.defaults):
        raise TypeError("transform must be an undecorated def transform(msg)")
    # Compilation checks e.g. top-level return, without running imports or code.
    compiled = compile(tree, "<python-node>", "exec")
    # Compile the entry separately: CPython may optimize away definitions
    # following a top-level raise in the whole module, but validation must not
    # execute or reject such code merely because it has side effects.
    entry_module = compile(ast.Module(body=[entry], type_ignores=[]), "<python-node>", "exec")
    function = next(item for item in entry_module.co_consts
                    if isinstance(item, types.CodeType) and item.co_name == "transform")
    if function.co_flags & (inspect.CO_GENERATOR | inspect.CO_COROUTINE | inspect.CO_ASYNC_GENERATOR):
        raise TypeError("transform must be synchronous and must not yield")
    return compiled


def strict_json(value, seen=None):
    if seen is None:
        seen = set()
    kind = type(value)
    if value is None or kind in (str, bool, int):
        return
    if kind is float:
        if not math.isfinite(value):
            raise ValueError("Non-finite numbers cannot be returned")
        return
    if kind not in (dict, list):
        raise TypeError("Only JSON dictionaries, lists and primitive values are supported")
    if id(value) in seen:
        raise ValueError("Cyclic values cannot be returned")
    seen.add(id(value))
    if kind is dict:
        for key, item in value.items():
            if type(key) is not str:
                raise TypeError("JSON object keys must be strings")
            strict_json(item, seen)
    else:
        for item in value:
            strict_json(item, seen)
    seen.remove(id(value))


def dispatch(method, params):
    if method == "validate":
        validate_source(params["source"])
        return {"valid": True}
    if method == "imports":
        modules = params.get("modules", [])
        for module in modules:
            importlib.import_module(module)
        return {"valid": True, "modules": modules}
    if method != "execute":
        raise ValueError("Unknown method")
    msg = params["message"]
    if type(msg) is not dict:
        raise TypeError("Input message must be a dictionary")
    code = validate_source(params["source"])
    namespace = {"__name__": "__hmi_node__"}
    exec(code, namespace, namespace)
    transform = namespace.get("transform")
    if not inspect.isfunction(transform) or inspect.iscoroutinefunction(transform):
        raise TypeError("transform must be a synchronous function")
    result = transform(msg)
    if inspect.iscoroutine(result):
        result.close()
        raise TypeError("Coroutine results are not supported")
    if inspect.isgenerator(result) or inspect.isasyncgen(result):
        raise TypeError("Generator results are not supported")
    if result is not None and type(result) is not dict:
        raise TypeError("transform must return a dictionary or None")
    strict_json(result)
    encoded = json.dumps(result, ensure_ascii=False, allow_nan=False).encode("utf-8")
    if len(encoded) > MAX_MESSAGE_BYTES:
        raise ValueError("Output message exceeds 1 MiB")
    return {"message": result}


def main():
    output = isolate_output()
    request_id = None
    try:
        raw = sys.stdin.buffer.readline(4 * MAX_MESSAGE_BYTES + 1)
        if len(raw) > 4 * MAX_MESSAGE_BYTES:
            raise ValueError("Request exceeds 4 MiB")
        request = json.loads(raw.decode("utf-8"), parse_constant=lambda v: (_ for _ in ()).throw(ValueError(v)))
        request_id = request.get("id")
        if request.get("jsonrpc") != "2.0" or request_id != 1:
            raise ValueError("Invalid JSON-RPC request")
        response = {"jsonrpc": "2.0", "id": request_id,
                    "result": dispatch(request["method"], request.get("params", {}))}
    except BaseException as error:
        trace = traceback.format_exc()
        print(trace, file=sys.stderr)
        response = {"jsonrpc": "2.0", "id": request_id,
                    "error": {"code": -32000, "message": str(error),
                              "data": {"type": type(error).__name__, "traceback": trace}}}
    output.write(json.dumps(response, ensure_ascii=False, allow_nan=False) + "\n")
    output.flush()
    output.close()


if __name__ == "__main__":
    main()
