#!/usr/bin/env python3
"""Local-only channel preparation (Python 3.11+, standard library).

Never publishes, clones a tap, reads kubeconfigs or supplies placeholder hashes.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess
import tarfile
import tempfile
import tomllib

ROOT = Path(__file__).resolve().parents[2]
ARCHES = {"arm": "aarch64", "intel": "x86_64"}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def digest(path):
    require(path.is_file() and not path.is_symlink(), "Expected regular input file")
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def sha(value):
    require(re.fullmatch(r"[0-9a-fA-F]{64}", value) is not None,
            "Expected exactly 64 hexadecimal SHA256 characters")
    return value.lower()


def version_check(version):
    require(re.fullmatch(r"(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)", version),
            "Expected numeric X.Y.Z version")
    require(tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]["package"]["version"] == version,
            "Version must match Cargo workspace")


def checksums(path):
    require(not path.is_symlink(), "Checksum input must not be a symlink")
    result = {}
    for line in path.read_text().splitlines():
        if not line.strip():
            continue
        match = re.fullmatch(r"([0-9a-fA-F]{64}) [ *]([^/\\\s]+)", line)
        require(match is not None, "Expected sha256sum lines with basename-only filenames")
        value, name = match.groups()
        require(name not in result, "Duplicate checksum filename")
        result[name] = sha(value)
    require(result, "Empty checksum input")
    return result


def cask(version, sums, artifacts=None):
    version_check(version)
    hashes = {}
    for brew_arch, artifact_arch in ARCHES.items():
        name = f"Iceon_{version}_{artifact_arch}.dmg"
        require(name in sums or artifacts is not None, "Missing checksum for " + name)
        actual = digest(artifacts / name) if artifacts is not None else sums[name]
        if name in sums:
            require(actual == sums[name], "Artifact/checksum mismatch for " + name)
        hashes[brew_arch] = sha(actual)
    return f'''cask "iceon" do
  version "{version}"
  arch arm: "aarch64", intel: "x86_64"

  on_arm do
    sha256 "{hashes['arm']}"
  end
  on_intel do
    sha256 "{hashes['intel']}"
  end

  url "https://github.com/yan-ad/iceon/releases/download/#{{version}}/Iceon_#{{version}}_#{{arch}}.dmg"
  name "Iceon"
  desc "Native Kubernetes workspace"
  homepage "https://github.com/yan-ad/iceon"

  app "Iceon.app"
end
'''


def allowed_source(name):
    path = Path(name)
    if path.is_absolute() or ".." in path.parts or any(p.startswith(".") for p in path.parts):
        return False
    if any(p.lower() in {"target", "credentials", "secrets", "kubeconfig", "kubeconfigs", "vendor"}
           for p in path.parts):
        return False
    if path.suffix.lower() in {".pem", ".key", ".p12", ".pfx", ".crt", ".env"}:
        return False
    if name in {"Cargo.toml", "Cargo.lock", "LICENSE", "assets/icons/icon.png",
                "assets/icons/icon.icns", "assets/icons/icon.ico", "assets/icons/32x32.png"}:
        return True
    if name.startswith("packaging/linux/"):
        return path.name in {"ad.yan.iceon.desktop", "ad.yan.iceon.metainfo.xml"}
    if name.startswith("themes/"):
        return path.suffix == ".json"
    if len(path.parts) >= 3 and path.parts[0] == "crates":
        return (path.parts[2] in {"src", "assets"} or path.name in {"Cargo.toml", "build.rs"}) and path.suffix in {
            ".rs", ".toml", ".json", ".sql", ".txt", ".ttf", ".otf", ".svg", ".png", ".yaml", ".yml"}
    return False


def copy_source(root, destination):
    """Only tracked build inputs; never recursively copy a user's worktree/home."""
    names = subprocess.check_output(["git", "ls-files", "-z"], cwd=root).decode().split("\0")
    # Linux metadata can be new/untracked during local preparation.
    names += ["packaging/linux/ad.yan.iceon.desktop", "packaging/linux/ad.yan.iceon.metainfo.xml"]
    for name in sorted(set(names)):
        if not allowed_source(name):
            continue
        source = root / name
        require(all(not p.is_symlink() for p in (source, *source.parents)), "Symlink source forbidden")
        require(source.is_file(), "Missing tracked build input")
        target = destination / name
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, target)
    require((destination / "Cargo.lock").is_file(), "Missing Cargo.lock")


def manifest():
    return json.loads((ROOT / "packaging/flatpak/ad.yan.iceon.json").read_text())


def prepare_flatpak(stage, allow_network=False):
    source = stage / "source"
    source.mkdir()
    copy_source(ROOT, source)
    command = ["cargo", "vendor", "--locked", "--versioned-dirs"]
    if not allow_network:
        command.append("--offline")
    command.append("vendor")
    config = subprocess.check_output(command, cwd=source, text=True)
    parsed = tomllib.loads(config)
    require(set(parsed) == {"source"}, "Unexpected cargo vendor config")
    require(parsed["source"]["vendored-sources"]["directory"] == "vendor", "Vendor path must be relative")
    (source / ".cargo").mkdir()
    (source / ".cargo/config.toml").write_text(config)
    subprocess.run(["cargo", "metadata", "--frozen", "--offline", "--format-version", "1"],
                   cwd=source, check=True, stdout=subprocess.DEVNULL)
    (stage / "ad.yan.iceon.json").write_text(json.dumps(manifest(), indent=2) + "\n")


def prepare_remote(stage, revision, archive, checksum, cargo_sources):
    """Explicit source archive identity, never a mutable tag or unchecked URL."""
    require(re.fullmatch(r"[0-9a-f]{40}", revision), "Expected full lowercase source commit")
    require(digest(archive) == sha(checksum), "Source archive checksum mismatch")
    with tarfile.open(archive, "r:gz") as contents:
        lock = None
        seen = set()
        for index, member in enumerate(contents):
            require(index < 100000 and member.size <= 128 * 1024 * 1024, "Source archive exceeds limits")
            parts = member.name.rstrip("/").split("/")
            require(parts[0] == "iceon-" + revision
                    and all(p not in {"", ".", ".."} for p in parts)
                    and "\\" not in member.name, "Unsafe source archive path")
            require(member.name not in seen and (member.isfile() or member.isdir()),
                    "Duplicate or nonregular source archive member")
            seen.add(member.name)
            if parts[1:] == ["Cargo.lock"]:
                require(member.isfile() and member.size <= 4 * 1024 * 1024, "Invalid source lockfile")
                lock = tomllib.loads(contents.extractfile(member).read().decode())
        require(lock is not None, "Source archive missing Cargo.lock")
    expected = {}
    for package in lock["package"]:
        if "source" not in package:
            continue
        require(package["source"] == "registry+https://github.com/rust-lang/crates.io-index",
                "Remote preparation currently supports registry dependencies only")
        name, version = package["name"], package["version"]
        expected[f"https://static.crates.io/crates/{name}/{name}-{version}.crate"] = sha(package["checksum"])
    sources = json.loads(cargo_sources.read_text())
    require(isinstance(sources, list) and sources, "Expected generated Cargo source array")
    actual = {}
    for item in sources:
        require(isinstance(item, dict) and item.get("type") in {"archive", "inline"},
                "Only generated archive/inline sources supported; review git dependency sources separately")
        destination = item.get("dest", "")
        require(re.fullmatch(r"cargo/vendor/[A-Za-z0-9_.+-]+", destination)
                or destination == "cargo", "Unexpected generated source destination")
        if item["type"] == "archive":
            require(set(item) <= {"type", "archive-type", "url", "sha256", "dest"}
                    and destination != "cargo", "Unexpected archive options")
            sha(item.get("sha256", ""))
            require(re.fullmatch(r"https://static\.crates\.io/crates/[A-Za-z0-9_-]+/[A-Za-z0-9_.+-]+\.crate",
                                 item.get("url", "")),
                    "Expected canonical crates.io source")
            require(item["url"] not in actual, "Duplicate crate archive")
            actual[item["url"]] = sha(item["sha256"])
        else:
            require(set(item) == {"type", "contents", "dest", "dest-filename"},
                    "Unexpected inline options")
            require(item["dest-filename"] == ("config" if destination == "cargo" else ".cargo-checksum.json"),
                    "Unexpected inline filename")
            require(isinstance(item["contents"], str), "Expected inline text")
            if destination == "cargo":
                config = tomllib.loads(item["contents"])
                require(config == {"source": {
                    "vendored-sources": {"directory": "cargo/vendor"},
                    "crates-io": {"replace-with": "vendored-sources"}}},
                    "Expected registry-only generated Cargo config")
    require(sum(item.get("dest") == "cargo" for item in sources) == 1,
            "Missing or duplicate generated Cargo config")
    require(actual == expected, "Generated crate sources must exactly match archive Cargo.lock")
    data = manifest()
    data["modules"][0]["sources"] = [{
        "type": "archive",
        "url": f"https://github.com/yan-ad/iceon/archive/{revision}.tar.gz",
        "sha256": sha(checksum),
    }, "cargo-sources.json"]
    data["modules"][0]["build-options"]["env"]["CARGO_HOME"] = "/run/build/iceon/cargo"
    (stage / "cargo-sources.json").write_text(json.dumps(sources, indent=2) + "\n")
    (stage / "ad.yan.iceon.json").write_text(json.dumps(data, indent=2) + "\n")


def export(output, writer):
    output = output.absolute()
    require(not output.exists() and not output.is_symlink(), "Output directory must not exist")
    require(all(not p.is_symlink() for p in output.parents), "Symlink output parent forbidden")
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=".iceon-channel-", dir=output.parent) as temporary:
        stage = Path(temporary) / "export"
        stage.mkdir()
        writer(stage)
        stage.rename(output)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    brew = commands.add_parser("homebrew")
    brew.add_argument("--version", required=True)
    brew.add_argument("--checksums", type=Path)
    brew.add_argument("--artifacts", type=Path)
    brew.add_argument("--output", type=Path, required=True)
    flatpak = commands.add_parser("flatpak")
    flatpak.add_argument("--allow-network", action="store_true", help="Allow cargo vendor downloads during preparation only")
    flatpak.add_argument("--output", type=Path, required=True)
    remote = commands.add_parser("flatpak-remote")
    remote.add_argument("--revision", required=True)
    remote.add_argument("--source-archive", type=Path, required=True)
    remote.add_argument("--source-sha256", required=True)
    remote.add_argument("--cargo-sources", type=Path, required=True)
    remote.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.command == "homebrew":
        require(args.checksums or args.artifacts, "Real artifact directory or checksum file required")
        text = cask(args.version, checksums(args.checksums) if args.checksums else {}, args.artifacts)
        def write_brew(stage):
            (stage / "Casks").mkdir()
            (stage / "Casks/iceon.rb").write_text(text)
            (stage / "README.md").write_text((ROOT / "packaging/homebrew/tap-README.md").read_text())
        export(args.output, write_brew)
    elif args.command == "flatpak":
        export(args.output, lambda stage: prepare_flatpak(stage, args.allow_network))
    else:
        export(args.output, lambda stage: prepare_remote(stage, args.revision, args.source_archive,
                                                      args.source_sha256, args.cargo_sources))
    print("CHANNEL_PREPARATION_OK: " + str(args.output))


if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        raise SystemExit("Channel preparation failed: " + str(error)) from None
