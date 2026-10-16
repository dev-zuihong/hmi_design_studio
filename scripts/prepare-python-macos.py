#!/usr/bin/env python3
"""Prepare a pinned python-build-standalone macOS runtime without system Python."""
import argparse
import email
import hashlib
import json
import os
import platform
import subprocess
import sys
import shutil
import tarfile
import tempfile
import time
import urllib.request
import zipfile
from pathlib import Path
from pathlib import PurePosixPath

from pip._vendor.packaging.markers import default_environment
from pip._vendor.packaging.requirements import Requirement
from pip._vendor.packaging.tags import mac_platforms
from pip._vendor.packaging.utils import canonicalize_name, parse_wheel_filename
from pip._vendor.packaging.version import Version

SOURCE_FILE = Path(__file__).resolve().parents[1] / "resources/python/runtime-source-macos.json"
SOURCE = json.loads(SOURCE_FILE.read_text("utf-8"))
VERSION = SOURCE["version"]
RELEASE = SOURCE["release"]
TARGETS = {"arm64": "aarch64-apple-darwin", "x86_64": "x86_64-apple-darwin"}


def sha256(path):
    digest = hashlib.sha256()
    with open(path, "rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def copy_symlinks_as_files(root):
    # The standalone archive uses aliases such as bin/python3 -> python3.14.
    # Materialize them so runtime verification can reject every symlink.
    for directory, dirs, files in os.walk(root, topdown=True, followlinks=False):
        for name in list(dirs) + files:
            path = Path(directory) / name
            if path.is_symlink():
                target = path.resolve(strict=True)
                if not target.is_file():
                    raise RuntimeError(f"Unsupported runtime symlink: {path}")
                path.unlink()
                shutil.copy2(target, path)


def wheel_tags_compatible(tags, target):
    architecture = "arm64" if target.startswith("aarch64") else "x86_64"
    supported_platforms = set(mac_platforms(version=(14, 0), arch=architecture))
    for tag in tags:
        if tag.platform == "any" and tag.abi == "none" and tag.interpreter in ("py3", "py314", "cp314"):
            return True
        if tag.platform not in supported_platforms:
            continue
        if tag.interpreter == "cp314" and tag.abi in ("cp314", "abi3", "none"):
            return True
        if tag.abi == "abi3" and tag.interpreter.startswith("cp3"):
            minor = tag.interpreter[3:]
            if minor.isdigit() and int(minor) <= 14:
                return True
    return False


def inspect_wheel(path, target):
    name, version, _, tags = parse_wheel_filename(path.name)
    platform_tags = {tag.platform for tag in tags}
    valid = wheel_tags_compatible(tags, target)
    if not valid:
        raise ValueError(f"Incompatible wheel for CPython 3.14 / {target}: {path.name} ({platform_tags})")
    with zipfile.ZipFile(path) as bundle:
        metadata_paths = [item for item in bundle.namelist() if item.endswith(".dist-info/METADATA")]
        if len(metadata_paths) != 1:
            raise ValueError(f"Expected one wheel METADATA: {path.name}")
        metadata = email.message_from_bytes(bundle.read(metadata_paths[0]))
    if canonicalize_name(metadata["Name"]) != canonicalize_name(name) or Version(metadata["Version"]) != version:
        raise ValueError(f"Wheel filename and metadata disagree: {path.name}")
    return {"name": canonicalize_name(name), "version": str(version), "wheel": path.name,
            "sha256": sha256(path), "tags": sorted(map(str, tags)), "requires": metadata.get_all("Requires-Dist", [])}


def dependency_closure(packages, roots, target):
    indexed = {item["name"]: item for item in packages}
    if len(indexed) != len(packages):
        raise ValueError("Conflicting wheel versions for one package")
    arm = target.startswith("aarch64")
    environment = default_environment()
    environment.update(python_version="3.14", python_full_version=VERSION, sys_platform="darwin",
                       platform_system="Darwin", platform_machine="arm64" if arm else "x86_64")
    seen, pending = set(), list(roots)
    while pending:
        requirement = pending.pop(0)
        if requirement.url:
            raise ValueError("Direct URL dependencies are not accepted")
        key = canonicalize_name(requirement.name)
        selected = indexed.get(key)
        if not selected or Version(selected["version"]) not in requirement.specifier:
            raise ValueError(f"Missing or incompatible transitive dependency: {requirement}")
        identity = (key, tuple(sorted(requirement.extras)))
        if identity in seen:
            continue
        seen.add(identity)
        for dependency in selected["requires"]:
            candidate = Requirement(dependency)
            if candidate.marker is None or any(candidate.marker.evaluate({**environment, "extra": extra}) for extra in (requirement.extras or {""})):
                pending.append(candidate)


def extract_wheel(path, destination, owned):
    with zipfile.ZipFile(path) as bundle:
        for member in bundle.infolist():
            if member.is_dir():
                continue
            pieces = PurePosixPath(member.filename).parts
            if not pieces or any(piece in ("..", ".") or ":" in piece or "\\" in piece for piece in pieces) or member.filename.startswith("/"):
                raise ValueError(f"Unsafe wheel entry: {member.filename}")
            if (member.external_attr >> 16) & 0o170000 == 0o120000:
                raise ValueError(f"Wheel symlinks are unsupported: {member.filename}")
            if pieces[0].endswith(".data"):
                if len(pieces) < 3 or pieces[1] not in ("purelib", "platlib"):
                    raise ValueError(f"Unsupported wheel installer behavior: {member.filename}")
                pieces = pieces[2:]
            if pieces[-1].lower().endswith(".pth"):
                raise ValueError(f"Wheel uses unsupported .pth initialization: {member.filename}")
            relative = "/".join(pieces)
            if relative.casefold() in owned:
                raise ValueError(f"Dependency file conflict: {relative}")
            owned.add(relative.casefold())
            target = destination.joinpath(*pieces)
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(bundle.read(member))


def prepare_dependencies(runtime, target, requirements, imports, output, wheelhouse):
    if sys.version_info[:2] != (3, 14):
        raise RuntimeError("Dependency preparation requires build-machine Python 3.14 with pip")
    roots, marker_env = [], default_environment()
    marker_env.update(python_version="3.14", python_full_version=VERSION,
                      sys_platform="darwin", platform_system="Darwin",
                      platform_machine="arm64" if target.startswith("aarch64") else "x86_64")
    for line in requirements.read_text("utf-8").splitlines():
        line = line.strip()
        if line and not line.startswith("#"):
            requirement = Requirement(line)
            if requirement.url:
                raise ValueError("Use pinned package requirements, not direct URLs")
            if requirement.marker is None or requirement.marker.evaluate(marker_env):
                roots.append(requirement)
    if not roots:
        raise ValueError("No requirements apply to this macOS target")
    if output.exists():
        raise ValueError(f"Output already exists: {output}")
    output.parent.mkdir(parents=True, exist_ok=True)
    platform_tag = "macosx_14_0_arm64" if target.startswith("aarch64") else "macosx_14_0_x86_64"
    with tempfile.TemporaryDirectory(prefix="hmi-wheelhouse-") as wheel_temp:
        wheels_dir = Path(wheel_temp)
        command = [sys.executable, "-m", "pip", "download", "--disable-pip-version-check", "--only-binary=:all:",
                   "--platform", platform_tag, "--implementation", "cp", "--python-version", "3.14",
                   "--abi", "cp314", "--abi", "abi3", "--abi", "none", "--dest", str(wheels_dir)]
        if wheelhouse:
            command += ["--no-index", "--find-links", str(wheelhouse.resolve())]
        subprocess.run(command + [str(item) for item in roots], check=True)
        wheel_files = sorted(wheels_dir.glob("*.whl"))
        packages = [inspect_wheel(path, target) for path in wheel_files]
        dependency_closure(packages, roots, target)
        with tempfile.TemporaryDirectory(prefix=".hmi-deps-", dir=output.parent) as staging_root:
            staged = Path(staging_root) / "runtime"
            shutil.copytree(runtime, staged)
            owned = set()
            for wheel in wheel_files:
                extract_wheel(wheel, staged / "dependencies", owned)
            lock = {"schemaVersion": 1, "pythonVersion": VERSION, "platform": target, "abi": "cp314",
                    "requirements": [str(item) for item in roots], "packages": packages, "imports": imports}
            (staged / "dependencies.lock.json").write_text(json.dumps(lock, indent=2) + "\n", "utf-8")
            manifest = json.loads((staged / "runtime-manifest.json").read_text("utf-8"))
            manifest["files"] = {path.relative_to(staged).as_posix(): sha256(path)
                                 for path in sorted(staged.rglob("*")) if path.is_file() and path.name != "runtime-manifest.json" and path.suffix != ".pyc" and "__pycache__" not in path.parts}
            (staged / "runtime-manifest.json").write_text(json.dumps(manifest, indent=2) + "\n", "utf-8")
            staged.rename(output)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--architecture", choices=TARGETS, default="arm64" if platform.machine() in ("arm64", "aarch64") else "x86_64")
    parser.add_argument("--cache-directory", type=Path)
    parser.add_argument("--archive-path", type=Path)
    parser.add_argument("--requirements", type=Path)
    parser.add_argument("--imports", nargs="+")
    parser.add_argument("--output", type=Path, help="Dependency-bearing runtime output; requires --requirements and --imports")
    parser.add_argument("--wheelhouse", type=Path, help="Optional local wheelhouse for offline preparation")
    args = parser.parse_args()
    if args.requirements or args.imports or args.output:
        if not (args.requirements and args.imports and args.output):
            raise SystemExit("--requirements, --imports, and --output must be passed together")
    target = TARGETS[args.architecture]
    artifact_source = SOURCE["artifacts"][target]
    expected_digest = artifact_source["sha256"]
    project = Path(__file__).resolve().parents[1]
    cache = (args.cache_directory or project / ".cache/python").resolve()
    cache.mkdir(parents=True, exist_ok=True)
    artifact = f"cpython-{VERSION}+{RELEASE}-{target}-install_only.tar.gz"
    archive = args.archive_path or cache / artifact
    if not archive.exists():
        url = artifact_source["url"]
        urllib.request.urlretrieve(url, archive)
    digest = sha256(archive)
    if digest != expected_digest:
        raise SystemExit(f"Archive SHA-256 mismatch: {digest}")
    destination = cache / f"{VERSION}-{target}"
    if destination.exists():
        manifest = json.loads((destination / "runtime-manifest.json").read_text())
        if manifest.get("archiveSha256") != digest or manifest.get("platform") != target:
            raise SystemExit(f"Existing runtime does not match the pinned archive: {destination}")
        changed = []
        for relative, checksum in manifest["files"].items():
            path = destination / relative
            if not path.resolve().is_relative_to(destination.resolve()):
                raise SystemExit(f"Invalid runtime manifest path: {relative}")
            if sha256(path) != checksum:
                changed.append(relative)
        actual = {path.relative_to(destination).as_posix() for path in destination.rglob("*") if path.is_file() and path.name != "runtime-manifest.json" and path.suffix != ".pyc" and "__pycache__" not in path.parts}
        if actual != set(manifest["files"]):
            raise SystemExit("Existing runtime contains files outside its locked manifest")
        lock = json.loads((destination / "dependencies.lock.json").read_text("utf-8"))
        if (lock.get("schemaVersion"), lock.get("pythonVersion"), lock.get("platform"), lock.get("abi"), lock.get("packages"), lock.get("imports")) != (1, VERSION, target, "cp314", [], []):
            raise SystemExit("Existing runtime dependency lock is not the pristine pinned lock")
        worker_digest = sha256(project / "resources/python/worker.py")
        if changed and changed != ["executor/worker.py"]:
            raise SystemExit(f"Existing runtime files failed verification: {', '.join(changed)}")
        if manifest["files"].get("executor/worker.py") != worker_digest:
            if changed:
                raise SystemExit("Cached worker does not match its own manifest; refusing refresh")
            # Only refresh the executor when every archived file and the exact
            # empty dependency lock still match the pinned release. Keep the
            # prior directory for recovery and restore it if installation fails.
            with tempfile.TemporaryDirectory(prefix=".hmi-python-refresh-", dir=cache) as temporary:
                staged = Path(temporary) / "runtime"
                shutil.copytree(destination, staged)
                shutil.copy2(project / "resources/python/worker.py", staged / "executor/worker.py")
                manifest["files"]["executor/worker.py"] = worker_digest
                (staged / "runtime-manifest.json").write_text(json.dumps(manifest, indent=2) + "\n", "utf-8")
                previous = cache / f"previous-{VERSION}-{target}-{int(time.time())}"
                destination.rename(previous)
                try:
                    staged.rename(destination)
                except BaseException:
                    previous.rename(destination)
                    raise
            print(f"Refreshed executor; previous verified runtime kept at {previous}")
        elif changed:
            raise SystemExit(f"Existing runtime files failed verification: {', '.join(changed)}")
        print(f"Reusing verified runtime: {destination}")
        if args.requirements or args.output:
            if not (args.requirements and args.imports and args.output):
                raise SystemExit("--requirements, --imports, and --output must be passed together")
            prepare_dependencies(destination, target, args.requirements.resolve(), args.imports,
                                 args.output.resolve(), args.wheelhouse)
        return
    temporary_context = tempfile.TemporaryDirectory(prefix=".hmi-python-", dir=cache)
    temporary_root = Path(temporary_context.name)
    extracted = temporary_root / "archive"
    extracted.mkdir()
    with tarfile.open(archive, "r:gz") as bundle:
        for member in bundle.getmembers():
            member_path = Path(member.name)
            if member_path.is_absolute() or ".." in member_path.parts:
                raise SystemExit(f"Unsafe archive path: {member.name}")
        bundle.extractall(extracted, filter="data")
    source = extracted / "python"
    if not source.is_dir():
        raise SystemExit("Standalone archive is missing its python/ directory")
    copy_symlinks_as_files(source)
    staged = temporary_root / "runtime"
    shutil.copytree(source, staged)
    executor = staged / "executor"
    executor.mkdir()
    shutil.copy2(project / "resources/python/worker.py", executor / "worker.py")
    dependencies = staged / "dependencies"
    dependencies.mkdir()
    lock = {"schemaVersion": 1, "pythonVersion": VERSION, "platform": target, "abi": "cp314", "packages": [], "imports": []}
    (staged / "dependencies.lock.json").write_text(json.dumps(lock, indent=2) + "\n")
    files = {path.relative_to(staged).as_posix(): sha256(path) for path in sorted(staged.rglob("*")) if path.is_file() and path.suffix != ".pyc" and "__pycache__" not in path.parts}
    manifest = {"schemaVersion": 1, "version": VERSION, "platform": target, "abi": "cp314", "archiveSha256": digest, "files": files}
    (staged / "runtime-manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    staged.rename(destination)
    temporary_context.cleanup()
    print(f"Prepared Python runtime: {destination}")
    if args.requirements or args.output:
        if not (args.requirements and args.imports and args.output):
            raise SystemExit("--requirements, --imports, and --output must be passed together")
        prepare_dependencies(destination, target, args.requirements.resolve(), args.imports,
                             args.output.resolve(), args.wheelhouse)


if __name__ == "__main__":
    main()
