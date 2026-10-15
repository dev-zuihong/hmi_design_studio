"""Prepare a verified offline runtime from binary wheels, using build-machine pip.

Run with standard CPython 3.14 on Windows x64 (not the embeddable runtime):
  python scripts/prepare-python-deps.py --runtime .cache/python/3.14.8-win_amd64 \
      --requirements requirements.txt --imports numpy --output .cache/python/project-env
Pass --wheelhouse to prepare without accessing the network. The output is a
complete relocatable environment; existing environments are never overwritten.
"""
import argparse
import email
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import platform
import shutil
import subprocess
import sys
import tempfile
import zipfile

# This tool runs on the build machine, where pip is explicitly required.
from pip._vendor.packaging.markers import default_environment
from pip._vendor.packaging.requirements import Requirement
from pip._vendor.packaging.utils import canonicalize_name, parse_wheel_filename
from pip._vendor.packaging.version import Version


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def check_tag(tag):
    if tag.platform == "any" and tag.abi == "none":
        return tag.interpreter in ("py3", "py314", "cp314")
    if tag.platform != "win_amd64":
        return False
    if tag.interpreter == "cp314" and tag.abi in ("cp314", "abi3", "none"):
        return True
    if tag.abi == "abi3" and tag.interpreter.startswith("cp3"):
        minor = tag.interpreter[3:]
        return minor.isdigit() and int(minor) <= 14
    return False


def inspect_wheel(path):
    name, version, _, tags = parse_wheel_filename(path.name)
    if not any(check_tag(tag) for tag in tags):
        raise ValueError(f"Incompatible wheel for CPython 3.14 / win_amd64: {path.name}")
    with zipfile.ZipFile(path) as archive:
        metadata_paths = [item for item in archive.namelist() if item.endswith(".dist-info/METADATA")]
        if len(metadata_paths) != 1:
            raise ValueError(f"Expected exactly one METADATA: {path.name}")
        metadata = email.message_from_bytes(archive.read(metadata_paths[0]))
        if canonicalize_name(metadata["Name"]) != canonicalize_name(name) or Version(metadata["Version"]) != version:
            raise ValueError(f"Wheel filename and metadata disagree: {path.name}")
    return {"name": canonicalize_name(name), "version": str(version), "wheel": path.name,
            "sha256": digest(path), "tags": sorted(map(str, tags)),
            "requires": metadata.get_all("Requires-Dist", [])}


def dependency_closure(packages, roots):
    indexed = {item["name"]: item for item in packages}
    if len(indexed) != len(packages):
        raise ValueError("Conflicting wheel versions for one package")
    environment = default_environment()
    environment.update(python_version="3.14", python_full_version="3.14.8",
                       sys_platform="win32", platform_system="Windows", platform_machine="AMD64")
    seen = set()
    pending = list(roots)
    while pending:
        requirement = pending.pop(0)
        if requirement.url:
            raise ValueError("Direct URL dependencies are not accepted in offline locks")
        key = canonicalize_name(requirement.name)
        identity = (key, tuple(sorted(requirement.extras)))
        selected = indexed.get(key)
        if not selected or Version(selected["version"]) not in requirement.specifier:
            raise ValueError(f"Missing or incompatible transitive dependency: {requirement}")
        if identity in seen:
            continue
        seen.add(identity)
        for dependency in selected["requires"]:
            candidate = Requirement(dependency)
            if candidate.marker is None or any(candidate.marker.evaluate({**environment, "extra": extra})
                                               for extra in (requirement.extras or {""})):
                pending.append(candidate)


def extract_wheel(path, destination, owned):
    with zipfile.ZipFile(path) as archive:
        for member in archive.infolist():
            if member.is_dir():
                continue
            pieces = PurePosixPath(member.filename).parts
            if not pieces or any(piece in ("..", ".") or ":" in piece or "\\" in piece for piece in pieces) or member.filename.startswith("/"):
                raise ValueError(f"Unsafe wheel entry: {member.filename}")
            if (member.external_attr >> 16) & 0o170000 == 0o120000:
                raise ValueError(f"Wheel symlinks are unsupported: {member.filename}")
            if pieces[0].endswith(".data"):
                if len(pieces) < 3 or pieces[1] not in ("purelib", "platlib"):
                    raise ValueError(f"Wheel requires unsupported installer behavior: {member.filename}")
                pieces = pieces[2:]
            if pieces[-1].lower().endswith(".pth"):
                raise ValueError(f"Wheel requires site/.pth initialization and must be adapted: {member.filename}")
            relative = "/".join(pieces)
            if relative.casefold() in owned:
                raise ValueError(f"Conflicting dependency file: {relative}")
            owned.add(relative.casefold())
            target = destination.joinpath(*pieces)
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(archive.read(member))


def verify_runtime(runtime):
    manifest = json.loads((runtime / "runtime-manifest.json").read_text("utf-8"))
    if (manifest["version"], manifest["platform"], manifest["abi"]) != ("3.14.8", "win_amd64", "cp314"):
        raise ValueError("Runtime version/platform/ABI mismatch")
    for relative, expected in manifest["files"].items():
        path = (runtime / relative).resolve()
        if not path.is_relative_to(runtime) or digest(path) != expected:
            raise ValueError(f"Runtime file validation failed: {relative}")
    if json.loads((runtime / "dependencies.lock.json").read_text("utf-8"))["packages"]:
        raise ValueError("Start from a clean standard-library runtime, not another project's dependencies")
    return manifest


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--runtime", type=Path, required=True)
    parser.add_argument("--requirements", type=Path, required=True)
    parser.add_argument("--imports", nargs="+", required=True, help="Module names verified by the limited runtime before preview/export")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--wheelhouse", type=Path, help="Offline wheel source")
    args = parser.parse_args()
    if sys.version_info[:2] != (3, 14) or sys.platform != "win32" or platform.machine().upper() not in ("AMD64", "X86_64"):
        raise RuntimeError("Prepare with standard CPython 3.14 on Windows x64 so pip resolves the correct environment markers")
    runtime = args.runtime.resolve()
    output = args.output.resolve()
    if output.exists():
        raise ValueError("Output already exists; choose a new directory")
    manifest = verify_runtime(runtime)
    roots = []
    environment = default_environment()
    environment.update(python_version="3.14", python_full_version="3.14.8")
    for line in args.requirements.read_text("utf-8").splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        req = Requirement(line)
        if req.url:
            raise ValueError("Use package requirements and a wheelhouse, not direct URLs")
        if req.marker is None or req.marker.evaluate(environment):
            roots.append(req)
    if not roots:
        raise ValueError("At least one explicit dependency is required")
    with tempfile.TemporaryDirectory(prefix="hmi-wheels-") as temporary:
        wheels = Path(temporary)
        command = [sys.executable, "-m", "pip", "download", "--disable-pip-version-check", "--only-binary=:all:",
                   "--platform", "win_amd64", "--implementation", "cp", "--python-version", "3.14",
                   "--abi", "cp314", "--abi", "abi3", "--abi", "none", "--dest", str(wheels)]
        if args.wheelhouse:
            command += ["--no-index", "--find-links", str(args.wheelhouse.resolve())]
        subprocess.run(command + [str(req) for req in roots], check=True)
        artifacts = sorted(wheels.glob("*.whl"))
        packages = [inspect_wheel(path) for path in artifacts]
        dependency_closure(packages, roots)
        # Build atomically in the destination's parent; no partially prepared
        # directory can be mistaken for a ready environment by the designer.
        output.parent.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(prefix=".hmi-env-", dir=output.parent) as staging:
            staged = Path(staging) / "runtime"
            shutil.copytree(runtime, staged)
            owned = set()
            for wheel in artifacts:
                extract_wheel(wheel, staged / "dependencies", owned)
            lock = {"schemaVersion": 1, "pythonVersion": "3.14.8", "platform": "win_amd64", "abi": "cp314",
                    "requirements": [str(req) for req in roots], "packages": packages, "imports": args.imports}
            (staged / "dependencies.lock.json").write_text(json.dumps(lock, ensure_ascii=False, indent=2) + "\n", "utf-8")
            manifest["files"] = {path.relative_to(staged).as_posix(): digest(path)
                                 for path in sorted(staged.rglob("*")) if path.is_file() and path.name != "runtime-manifest.json"}
            (staged / "runtime-manifest.json").write_text(json.dumps(manifest, indent=2) + "\n", "utf-8")
            staged.rename(output)
    print(f"Prepared {output}. Import this directory in the designer; preview/export must pass validate_imports() first.")


if __name__ == "__main__":
    main()
