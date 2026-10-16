"""Safely extract and verify a macOS Runner ZIP on a Mac.

The verifier accepts only internal relative symlinks, preserves Unix execute
bits, checks the complete release-manifest tree and app signature, and never
overwrites or deletes an existing extraction directory.
"""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path, PurePosixPath
import plistlib
import re
import shutil
import signal
import stat
import subprocess
import sys
import tempfile
import zipfile


PLATFORMS = {
    "macos-arm64": ("aarch64-apple-darwin", "arm64"),
    "macos-x64": ("x86_64-apple-darwin", "x86_64"),
}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def safe_name(name):
    require(isinstance(name, str) and name and not name.startswith("/"), f"Invalid package path: {name!r}")
    require("\\" not in name and ":" not in name and "\x00" not in name,
            f"Invalid package path: {name!r}")
    pieces = name.rstrip("/").split("/")
    require(all(piece and piece not in (".", "..") and not piece.endswith((" ", "."))
                for piece in pieces), f"Invalid package path: {name!r}")
    return "/".join(pieces)


def safe_link(name, target):
    require(isinstance(target, str) and target and not target.startswith("/"),
            f"Symlink must be relative: {name} -> {target!r}")
    require("\\" not in target and ":" not in target and "\x00" not in target,
            f"Invalid symlink target: {name} -> {target!r}")
    stack = list(PurePosixPath(name).parent.parts)
    for part in PurePosixPath(target).parts:
        if part in ("", "."):
            continue
        if part == "..":
            require(stack, f"Symlink escapes the package: {name} -> {target}")
            stack.pop()
        else:
            stack.append(part)
    require(stack, f"Invalid symlink target: {name} -> {target}")


def read_json(path):
    require(path.stat().st_size <= 32 * 1024 * 1024, f"JSON exceeds 32 MiB: {path}")
    return json.loads(path.read_text("utf-8"))


def zip_inventory(bundle):
    files, directories, seen, symlinks = set(), set(), set(), {}
    items = {}
    for item in bundle.infolist():
        name = safe_name(item.filename)
        require(name.casefold() not in seen, f"Duplicate or case-colliding ZIP entry: {name}")
        seen.add(name.casefold())
        mode = item.external_attr >> 16
        kind = stat.S_IFMT(mode)
        if item.is_dir():
            require(kind in (0, stat.S_IFDIR), f"Invalid directory entry type: {name}")
            directories.add(name)
            items[name] = ("directory", item)
        elif kind == stat.S_IFLNK:
            require(item.file_size <= 4096, f"Symlink target is too long: {name}")
            target_bytes = bundle.read(item)
            try:
                target = target_bytes.decode("utf-8")
            except UnicodeDecodeError as error:
                raise ValueError(f"Symlink target is not UTF-8: {name}") from error
            safe_link(name, target)
            symlinks[name] = target
            files.add(name)
            items[name] = ("symlink", item)
        else:
            require(kind in (0, stat.S_IFREG), f"Unsupported ZIP entry type: {name}")
            files.add(name)
            items[name] = ("file", item)
        directories.update(str(parent) for parent in PurePosixPath(name).parents if str(parent) != ".")
    require(not files & directories, "A ZIP entry is both a file and a directory")
    for link in symlinks:
        require(not any(name.startswith(link + "/") for name in files | directories),
                f"ZIP entry has a symlink parent: {link}")
    return files, directories, items, symlinks


def extract_package(archive, destination, reuse):
    with zipfile.ZipFile(archive) as bundle:
        files, directories, items, symlinks = zip_inventory(bundle)
        require("release-manifest.json" in files, "ZIP is missing release-manifest.json")
        manifest_item = items["release-manifest.json"][1]
        require(manifest_item.file_size <= 32 * 1024 * 1024, "Release manifest exceeds 32 MiB")
        require(items["release-manifest.json"][0] == "file", "Release manifest must be a regular file")
        manifest = json.loads(bundle.read(manifest_item).decode("utf-8"))
        platform = manifest.get("platform")
        require(manifest.get("schemaVersion") == 1 and platform in PLATFORMS,
                "Expected release manifest schemaVersion=1 and a supported macOS platform")
        expected = manifest.get("files")
        require(isinstance(expected, dict) and "release-manifest.json" not in expected,
                "Release manifest has an invalid file map")
        for name, digest in expected.items():
            require(safe_name(name) == name, f"Non-canonical file path: {name!r}")
            require(isinstance(digest, str) and re.fullmatch(r"[0-9a-f]{64}", digest),
                    f"Invalid SHA-256 for {name}")
        require(files == set(expected) | {"release-manifest.json"},
                f"ZIP file set differs from release manifest: missing={sorted(set(expected)-files)[:8]}, "
                f"extra={sorted(files-set(expected)-{'release-manifest.json'})[:8]}")
        for link, target in symlinks.items():
            require(link in expected, f"Symlink is not covered by release manifest: {link}")
            require(hashlib.sha256(target.encode("utf-8")).hexdigest() == expected[link],
                    f"Symlink-text SHA-256 mismatch: {link}")
        if destination.exists():
            require(reuse and destination.is_dir(),
                    "Extraction directory already exists; choose a new directory or pass --reuse")
        else:
            require(not reuse, "--reuse requires an existing extraction directory")
            destination.mkdir(parents=True)
            for name in sorted(directories, key=lambda value: (value.count("/"), value)):
                (destination / name).mkdir(parents=True, exist_ok=True)
            for name, (kind, item) in sorted(items.items()):
                if kind != "file":
                    continue
                target_path = destination / name
                target_path.parent.mkdir(parents=True, exist_ok=True)
                mode = (item.external_attr >> 16) & 0o777
                with bundle.open(item) as source, target_path.open("xb") as output:
                    shutil.copyfileobj(source, output, 1024 * 1024)
                if mode:
                    target_path.chmod(mode)
            for name, (kind, item) in sorted(items.items()):
                if kind != "symlink":
                    continue
                target_path = destination / name
                target_path.parent.mkdir(parents=True, exist_ok=True)
                target_path.symlink_to(symlinks[name])
            for name in directories:
                path = destination / name
                if path.is_dir() and not path.is_symlink():
                    path.chmod(0o755)
        actual_files, actual_directories = file_tree(destination)
        require(actual_files == files and actual_directories == directories,
                "Extracted file/directory set differs from ZIP (including empty directories)")
        require(read_json(destination / "release-manifest.json") == manifest,
                "Extracted release manifest differs from ZIP manifest")
    validate_release_files(destination, expected)
    return manifest, len(files), len(directories)


def file_tree(directory):
    files, directories, pending = set(), set(), [directory]
    while pending:
        current = pending.pop()
        for entry in current.iterdir():
            mode = entry.lstat().st_mode
            relative = entry.relative_to(directory).as_posix()
            if stat.S_ISLNK(mode):
                files.add(relative)
            elif stat.S_ISDIR(mode):
                directories.add(relative)
                pending.append(entry)
            elif stat.S_ISREG(mode):
                files.add(relative)
            else:
                raise ValueError(f"Unsupported extracted entry: {entry}")
    return files, directories


def file_sha256(path):
    if path.is_symlink():
        return hashlib.sha256(os.readlink(path).encode("utf-8")).hexdigest()
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def path_sha256(path):
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def validate_release_files(directory, files):
    for name, digest in files.items():
        path = directory / name
        require(file_sha256(path) == digest, f"Release SHA-256 mismatch: {name}")


def verify_python(resources, project, target, expectation, native):
    flows = project.get("flows", [])
    contains_python = any(node.get("kind") == "python" for flow in flows for node in flow.get("nodes", []))
    runtime_root = resources / "runtime/python"
    require(type(project.get("python")) is dict if contains_python else project.get("python") is None,
            "Project Python configuration does not match its nodes")
    if expectation != "auto":
        require(contains_python == (expectation != "absent"),
                f"Unexpected Python inclusion; expected {expectation}")
    if not contains_python:
        require(not runtime_root.exists(), "A project without Python includes a code-node runtime")
        require(not native, "--native requires Python nodes")
        return {"included": False}

    require(runtime_root.is_dir(), "Python project is missing runtime/python")
    runtime_manifest_path = runtime_root / "runtime-manifest.json"
    runtime = read_json(runtime_manifest_path)
    target_triple = PLATFORMS[target][0]
    require((runtime.get("schemaVersion"), runtime.get("version"), runtime.get("platform"), runtime.get("abi"))
            == (1, "3.14.8", target_triple, "cp314"), "Python runtime version/platform/ABI mismatch")
    require(project["python"].get("runtimeId") == f"3.14.8-{target_triple}",
            "Project Python runtimeId does not match the packaged platform")
    require(project["python"].get("runtimeSha256") == runtime.get("archiveSha256"),
            "Project and runtime Python archive hashes differ")
    source_manifest = read_json(Path(__file__).resolve().parents[1] / "resources/python/runtime-source-macos.json")
    expected_archive = source_manifest.get("artifacts", {}).get(target_triple, {}).get("sha256")
    require(isinstance(expected_archive, str) and runtime.get("archiveSha256") == expected_archive,
            "Python runtime archive hash does not match the pinned macOS source manifest")
    runtime_files = runtime.get("files")
    require(isinstance(runtime_files, dict) and "runtime-manifest.json" not in runtime_files,
            "Python runtime manifest has an invalid file map")
    actual_files, _ = file_tree(runtime_root)
    require("runtime-manifest.json" in actual_files, "Python runtime manifest is missing")
    # PythonEnvironment intentionally ignores runtime-generated bytecode caches
    # when it verifies the pinned distribution manifest.
    managed_files = {name for name in actual_files
                     if name != "runtime-manifest.json"
                     and ".pyc" != PurePosixPath(name).suffix
                     and "__pycache__" not in PurePosixPath(name).parts}
    require(managed_files == set(runtime_files),
            "Python runtime file set differs from runtime-manifest.json")
    validate_release_files(runtime_root, runtime_files)
    for binary in ("bin/python3.14", "lib/libpython3.14.dylib"):
        path = runtime_root / binary
        require(path.is_file(), f"Python runtime binary is missing: {binary}")
        if binary == "bin/python3.14":
            require(path.stat().st_mode & 0o111, f"Python executable is missing execute bits: {binary}")
        verify_architecture(path, PLATFORMS[target][1])

    lock_path = runtime_root / "dependencies.lock.json"
    lock = read_json(lock_path)
    require((lock.get("schemaVersion"), lock.get("pythonVersion"), lock.get("platform"), lock.get("abi"))
            == (1, "3.14.8", target_triple, "cp314"), "Python dependency lock platform mismatch")
    require(lock.get("packages") == project["python"].get("dependencyLock"),
            "Project and packaged Python dependency locks differ")
    if expectation == "stdlib":
        require(lock.get("packages") == [], "Standard-library package contains third-party packages")
    result = {"included": True, "version": runtime["version"], "platform": target_triple,
              "packages": [package.get("name") for package in lock.get("packages", [])]}
    if native:
        expected_arch = PLATFORMS[target][1]
        extensions = sorted((runtime_root / "dependencies").rglob("*.so"))
        require(bool(extensions) and bool(lock.get("packages")),
                "Native Python package requires locked third-party packages and .so extensions")
        for extension in extensions:
            verify_architecture(extension, expected_arch)
        result["nativeExtensions"] = len(extensions)
    return result


def verify_architecture(binary, expected, *, thin=False):
    result = subprocess.run(["/usr/bin/lipo", "-archs", str(binary)], capture_output=True, text=True)
    require(result.returncode == 0, f"Not a valid macOS Mach-O binary: {binary}: {result.stderr.strip()}")
    architectures = set(result.stdout.split())
    require(expected in architectures,
            f"Binary does not support {expected}: {binary} ({sorted(architectures)})")
    if thin:
        require(architectures == {expected},
                f"Expected native thin {expected} binary, found {sorted(architectures)}: {binary}")


def verify_application(root, target):
    app = root / "HmiRunner.app"
    require(app.is_dir(), "Package is missing HmiRunner.app")
    executable = app / "Contents/MacOS/hmi-runner"
    require(executable.is_file() and executable.stat().st_mode & 0o111,
            "Runner executable is missing or not executable")
    expected_arch = PLATFORMS[target][1]
    verify_architecture(executable, expected_arch, thin=True)
    result = subprocess.run(["/usr/bin/codesign", "--verify", "--deep", "--strict", str(app)],
                            capture_output=True, text=True)
    require(result.returncode == 0, f"Runner app signature is invalid: {result.stderr.strip()}")
    plist = plistlib.loads((app / "Contents/Info.plist").read_bytes())
    require(plist.get("CFBundleExecutable") == "hmi-runner", "Runner Info.plist executable does not match")
    minimum = plist.get("LSMinimumSystemVersion")
    require(minimum == "14.0", f"Runner must declare macOS 14.0 as its minimum: {minimum!r}")
    return app, app / "Contents/Resources", executable


def verify_project_resources(resources, project, target):
    for asset in project.get("assets", []):
        relative = safe_name(asset.get("path"))
        require((resources / relative).is_file(), f"Project asset is missing: {relative}")
    protocols = {connection.get("protocolId") for connection in project.get("connections", [])}
    for protocol_id in protocols:
        require(isinstance(protocol_id, str) and re.fullmatch(r"[A-Za-z0-9_-]+", protocol_id),
                f"Invalid protocol ID in project: {protocol_id!r}")
        directory = resources / "plugins" / protocol_id
        manifest = read_json(directory / "manifest.json")
        require(manifest.get("target") == PLATFORMS[target][0],
                f"Plugin {protocol_id} target does not match {target}")
        connection_versions = {connection.get("protocolVersion") for connection in project.get("connections", [])
                               if connection.get("protocolId") == protocol_id}
        require(connection_versions <= {manifest.get("version")},
                f"Plugin {protocol_id} version does not match project")
        executable_name = safe_name(manifest.get("executable"))
        executable = directory / executable_name
        require(executable.is_file() and executable.stat().st_mode & 0o111,
                f"Plugin executable is missing or not executable: {protocol_id}/{executable_name}")
        verify_architecture(executable, PLATFORMS[target][1])


def smoke(cli, app_resources, flow, timeout):
    cli = cli.resolve(strict=True)
    require(cli.is_file() and cli.stat().st_mode & 0o111, "hmi-cli is missing or not executable")
    environment = os.environ.copy()
    environment["PATH"] = "/usr/bin:/bin"
    environment["PYTHONHOME"] = "/__hmi_verify_missing_pythonhome__"
    environment["PYTHONPATH"] = "/__hmi_verify_missing_pythonpath__"
    project = app_resources / "project.json"
    command = [str(cli), "smoke", "--project", str(project), "--resources", str(app_resources),
               "--packaged", "--flow", flow]
    with tempfile.TemporaryDirectory(prefix="hmi-macos-smoke-") as cwd:
        process = subprocess.Popen(command, cwd=cwd, env=environment, start_new_session=True,
                                   stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        try:
            stdout, stderr = process.communicate(timeout=timeout)
        except subprocess.TimeoutExpired:
            try:
                os.killpg(process.pid, signal.SIGTERM)
                process.communicate(timeout=5)
            except (ProcessLookupError, subprocess.TimeoutExpired):
                try:
                    os.killpg(process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                process.communicate(timeout=5)
            raise ValueError(f"Packaged CLI smoke exceeded {timeout} seconds") from None
    require(process.returncode == 0, f"Packaged CLI smoke failed: {stderr or stdout}")
    result = json.loads(stdout)
    require(result.get("success") is True, "CLI did not report successful packaged smoke")
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("archive", type=Path)
    parser.add_argument("--directory", type=Path, required=True,
                        help="Explicit new extraction directory retained for native Runner testing")
    parser.add_argument("--reuse", action="store_true", help="Verify an existing extraction without modifying it")
    parser.add_argument("--python", choices=("auto", "absent", "stdlib", "present"), default="auto")
    parser.add_argument("--native", action="store_true", help="Require native third-party Python extensions")
    parser.add_argument("--cli", type=Path, help="Native hmi-cli binary; runs a packaged headless smoke")
    parser.add_argument("--flow", default="flow_read")
    parser.add_argument("--timeout", type=float, default=120)
    parser.add_argument("--report", type=Path, help="Write UTF-8 JSON verification evidence")
    args = parser.parse_args()
    require(sys.platform == "darwin", "macOS package verification must run on macOS")
    require(math.isfinite(args.timeout) and args.timeout > 0, "Timeout must be finite and positive")
    archive, directory = args.archive.resolve(strict=True), args.directory.resolve()
    report_path = args.report.resolve() if args.report else None
    require(not archive.is_relative_to(directory), "ZIP must be outside the extraction directory")
    if report_path:
        require(not report_path.is_relative_to(directory) and report_path != archive,
                "Report must be outside the extraction and must not overwrite the ZIP")
    manifest, files, directories = extract_package(archive, directory, args.reuse)
    app, resources, executable = verify_application(directory, manifest["platform"])
    project = read_json(resources / "project.json")
    require(project.get("id") == manifest.get("projectId"), "Project ID does not match release manifest")
    contains_python = any(node.get("kind") == "python"
                          for flow in project.get("flows", []) for node in flow.get("nodes", []))
    require(type(manifest.get("includesPython")) is bool and manifest["includesPython"] == contains_python,
            "Release manifest Python flag does not match project nodes")
    verify_project_resources(resources, project, manifest["platform"])
    python = verify_python(resources, project, manifest["platform"], args.python, args.native)
    report = {"success": True, "archive": str(archive),
              "archiveSha256": path_sha256(archive),
              "archiveBytes": archive.stat().st_size, "directory": str(directory),
              "files": files, "directories": directories, "manifestVerified": True,
              "platform": manifest["platform"], "application": str(app),
              "runnerExecutable": str(executable), "python": python, "smokeExecuted": False}
    if args.cli:
        report["smoke"] = smoke(args.cli, resources, args.flow, args.timeout)
        report["smokeExecuted"] = True
    if report_path:
        report_path.parent.mkdir(parents=True, exist_ok=True)
        report_path.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", "utf-8")
    print(json.dumps({key: value for key, value in report.items() if key != "smoke"},
                     ensure_ascii=True, indent=2))


if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError, KeyError, zipfile.BadZipFile, subprocess.SubprocessError) as error:
        print(f"macOS package verification failed: {error}", file=sys.stderr)
        sys.exit(1)
