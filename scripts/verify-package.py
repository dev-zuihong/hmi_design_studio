"""Verify and retain an exported Windows x64 runtime directory.

Existing directories are never overwritten or deleted. --reuse verifies an
already extracted directory against this exact ZIP, including empty directories.
The optional CLI smoke uses only packaged resources, a separate working
directory, and a PATH containing Windows system directories only.
"""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import stat
import subprocess
import sys
import tempfile
import zipfile


def require(condition, message):
    if not condition:
        raise ValueError(message)


def sha256(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def safe_name(name):
    require(isinstance(name, str) and name and not name.startswith("/")
            and "\\" not in name and ":" not in name, f"Invalid package path: {name!r}")
    pieces = name.rstrip("/").split("/")
    require(all(piece and piece not in (".", "..") and not piece.endswith((" ", "."))
                for piece in pieces), f"Invalid package path: {name!r}")
    return "/".join(pieces)


def reject_link(path):
    metadata = path.lstat()
    require(not stat.S_ISLNK(metadata.st_mode)
            and not getattr(metadata, "st_file_attributes", 0) & 0x400,
            f"Package contains a symlink or NTFS reparse point: {path}")


def file_tree(directory):
    files, directories, pending = set(), set(), [directory]
    while pending:
        current = pending.pop()
        reject_link(current)
        for entry in current.iterdir():
            reject_link(entry)
            relative = entry.relative_to(directory).as_posix()
            if entry.is_dir():
                directories.add(relative)
                pending.append(entry)
            else:
                require(entry.is_file(), f"Unsupported package entry: {entry}")
                files.add(relative)
    return files, directories


def validate_files(directory, manifest, manifest_name):
    expected = manifest.get("files")
    require(isinstance(expected, dict), f"Missing file map in {manifest_name}")
    require(manifest_name not in expected, f"{manifest_name} must not include itself")
    for name, digest in expected.items():
        require(safe_name(name) == name, f"Non-canonical file path: {name!r}")
        require(isinstance(digest, str) and re.fullmatch(r"[0-9a-f]{64}", digest), f"Invalid SHA-256: {name}")
    files, _ = file_tree(directory)
    require(files == set(expected) | {manifest_name},
            f"File set differs from {manifest_name}: missing={sorted(set(expected) - files)[:8]}, "
            f"extra={sorted(files - set(expected) - {manifest_name})[:8]}")
    for name, digest in expected.items():
        require(sha256(directory / name) == digest, f"SHA-256 mismatch: {name}")


def read_json(path):
    require(path.stat().st_size <= 32 * 1024 * 1024, f"JSON exceeds 32 MiB: {path}")
    return json.loads(path.read_text("utf-8"))


def extract_package(archive, directory, reuse):
    with zipfile.ZipFile(archive) as bundle:
        files, directories, seen = set(), set(), set()
        for item in bundle.infolist():
            name = safe_name(item.filename)
            require(name.casefold() not in seen, f"Duplicate or case-colliding ZIP entry: {name}")
            seen.add(name.casefold())
            require(not stat.S_ISLNK(item.external_attr >> 16), f"ZIP symlink is unsupported: {name}")
            if item.is_dir():
                directories.add(name)
            else:
                files.add(name)
            directories.update(str(parent) for parent in PurePosixPath(name).parents if str(parent) != ".")
        require("release-manifest.json" in files, "ZIP is missing release-manifest.json")
        require(not files & directories, "A ZIP entry is both a file and a directory")
        require(bundle.getinfo("release-manifest.json").file_size <= 32 * 1024 * 1024, "Release manifest exceeds 32 MiB")
        manifest = json.loads(bundle.read("release-manifest.json").decode("utf-8"))
        require(manifest.get("schemaVersion") == 1 and manifest.get("platform") == "windows-x64",
                "Expected release manifest schemaVersion=1 and platform=windows-x64")
        require(isinstance(manifest.get("files"), dict)
                and files == set(manifest["files"]) | {"release-manifest.json"}, "ZIP file set does not match release manifest")
        if directory.exists():
            require(reuse, "Extraction directory already exists; choose a new directory or pass --reuse")
            require(directory.is_dir(), "Extraction destination is not a directory")
        else:
            require(not reuse, "--reuse requires an existing extracted directory")
            directory.mkdir(parents=True)
            # Paths and types have been checked before any ZIP content is written.
            for item in bundle.infolist():
                target = directory / safe_name(item.filename)
                if item.is_dir():
                    target.mkdir(parents=True, exist_ok=True)
                else:
                    target.parent.mkdir(parents=True, exist_ok=True)
                    with bundle.open(item) as source, target.open("xb") as output:
                        shutil.copyfileobj(source, output, 1024 * 1024)
        require(read_json(directory / "release-manifest.json") == manifest,
                "Extracted manifest differs from the ZIP manifest")
        actual_files, actual_directories = file_tree(directory)
        require(actual_files == files and actual_directories == directories,
                "Extracted file/directory set differs from the ZIP (including empty directories)")
    validate_files(directory, manifest, "release-manifest.json")
    return manifest, len(files), len(directories)


def verify_python(directory, project, manifest, expected, native):
    contains = any(node.get("kind") == "python" for flow in project.get("flows", []) for node in flow.get("nodes", []))
    require(type(manifest.get("includesPython")) is bool and manifest["includesPython"] == contains,
            "includesPython does not match project nodes (disabled flows also count)")
    if expected != "auto":
        require(contains == (expected != "absent"), f"Unexpected Python inclusion; expected {expected}")
    if not contains:
        require(not native, "--native requires Python nodes")
        require(not (directory / "runtime/python").exists(), "A project without Python includes a code-node runtime")
        require(project.get("python") is None, "A project without Python retains Python configuration")
        return {"included": False}
    environment = directory / "runtime/python"
    for name in ("python.exe", "python314.dll", "python314.zip", "python314._pth", "executor/worker.py", "dependencies.lock.json"):
        require((environment / name).is_file(), f"Python resource is missing: {name}")
    runtime = read_json(environment / "runtime-manifest.json")
    require((runtime.get("schemaVersion"), runtime.get("version"), runtime.get("platform"), runtime.get("abi"))
            == (1, "3.14.8", "win_amd64", "cp314"), "Python runtime version/platform/ABI mismatch")
    validate_files(environment, runtime, "runtime-manifest.json")
    lock = read_json(environment / "dependencies.lock.json")
    require(lock.get("packages") == project.get("python", {}).get("dependencyLock", []), "Project and packaged dependency locks differ")
    require(project.get("python", {}).get("runtimeSha256") == runtime.get("archiveSha256"), "Project/runtime Python archive hashes differ")
    if expected == "stdlib":
        require(lock["packages"] == [], "Standard-library package includes third-party dependencies")
    result = {"included": True, "version": runtime["version"],
              "packages": [{"name": item["name"], "version": item["version"]} for item in lock["packages"]]}
    if native:
        packages = {item["name"] for item in lock["packages"]}
        require({"numpy", "requests"} <= packages, "Native fixture requires locked numpy and requests")
        dependencies = environment / "dependencies"
        pyd = list((dependencies / "numpy").rglob("*.pyd"))
        dll = list((dependencies / "numpy.libs").rglob("*.dll"))
        licenses = [path for path in dependencies.rglob("*") if path.is_file() and ".dist-info/licenses/" in path.as_posix()]
        require(pyd and dll and licenses and (dependencies / "requests/__init__.py").is_file(),
                "Native fixture is missing NumPy extensions/DLLs, requests or dependency licenses")
        result.update(numpyExtensions=len(pyd), numpyDlls=len(dll), dependencyLicenses=len(licenses))
    return result


def smoke(cli, directory, flow, timeout):
    require(os.name == "nt", "Packaged CLI smoke requires Windows")
    environment = os.environ.copy()
    system = Path(environment.get("SystemRoot", "C:/Windows"))
    environment["PATH"] = str(system / "System32") + os.pathsep + str(system)
    environment["PYTHONHOME"] = str(directory / "__missing_system_python__")
    environment["PYTHONPATH"] = str(directory / "__missing_system_modules__")
    command = [str(cli), "smoke", "--project", str(directory / "project.json"), "--resources", str(directory), "--packaged", "--flow", flow]
    process = subprocess.Popen(command, cwd=tempfile.gettempdir(), env=environment,
                               stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                               text=True, encoding="utf-8", creationflags=subprocess.CREATE_NO_WINDOW)
    try:
        stdout, stderr = process.communicate(timeout=timeout)
    except subprocess.TimeoutExpired:
        if process.poll() is None:
            subprocess.run([str(system / "System32/taskkill.exe"), "/PID", str(process.pid), "/T", "/F"],
                           capture_output=True, timeout=10, creationflags=subprocess.CREATE_NO_WINDOW)
            process.kill()
        process.communicate(timeout=10)
        raise ValueError(f"Packaged smoke exceeded {timeout} seconds") from None
    require(process.returncode == 0, f"Packaged smoke failed: {stderr or stdout}")
    result = json.loads(stdout)
    require(result.get("success") is True, "CLI did not report successful packaged smoke")
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("archive", type=Path)
    parser.add_argument("--directory", type=Path, required=True, help="Explicit extraction directory retained for native GUI tests")
    parser.add_argument("--reuse", action="store_true", help="Verify an existing extraction; never overwrite it")
    parser.add_argument("--python", choices=("auto", "absent", "stdlib", "present"), default="auto")
    parser.add_argument("--native", action="store_true", help="Require NumPy/requests and native runtime files")
    parser.add_argument("--cli", type=Path, help="Absolute or relative path to the final built hmi-cli.exe; enables packaged smoke")
    parser.add_argument("--flow", default="flow_read")
    parser.add_argument("--timeout", type=float, default=120)
    parser.add_argument("--report", type=Path, help="Write UTF-8 JSON verification evidence outside the extracted directory")
    args = parser.parse_args()
    require(math.isfinite(args.timeout) and args.timeout > 0, "Timeout must be finite and positive")
    archive, directory = args.archive.resolve(strict=True), args.directory.resolve()
    report_path = args.report.resolve() if args.report else None
    require(not archive.is_relative_to(directory), "ZIP must be outside the extraction directory")
    if report_path:
        require(not report_path.is_relative_to(directory) and report_path != archive,
                "Report must be outside the extraction and must not overwrite the ZIP")
    manifest, files, directories = extract_package(archive, directory, args.reuse)
    for name in ("HmiRunner.exe", "START.cmd", "project.json", "webview2/msedgewebview2.exe"):
        require((directory / name).is_file(), f"Runtime package is missing: {name}")
    project = read_json(directory / "project.json")
    require(project.get("id") == manifest.get("projectId"), "Project ID does not match the release manifest")
    python = verify_python(directory, project, manifest, args.python, args.native)
    report = {"success": True, "archive": str(archive), "archiveSha256": sha256(archive),
              "archiveBytes": archive.stat().st_size, "directory": str(directory), "files": files,
              "directories": directories, "manifestVerified": True, "python": python, "smokeExecuted": False}
    if args.cli:
        report["smoke"] = smoke(args.cli.resolve(strict=True), directory, args.flow, args.timeout)
        report["smokeExecuted"] = True
    if report_path:
        report_path.parent.mkdir(parents=True, exist_ok=True)
        report_path.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", "utf-8")
    print(json.dumps({key: value for key, value in report.items() if key != "smoke"}, ensure_ascii=True, indent=2))


if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError, KeyError, zipfile.BadZipFile, subprocess.SubprocessError) as error:
        print(f"Package verification failed: {error}", file=sys.stderr)
        sys.exit(1)
