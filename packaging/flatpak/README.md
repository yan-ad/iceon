# Experimental Flatpak preparation, not shipped sandbox support

Identity: `ad.yan.iceon`. Checked-in JSON uses a **local staged directory**, not a
remote source with missing checksums. It is runnable only after preparation.
No Flathub submission, remote repository creation or publication occurs.

## Local vendored source build

On Linux, install Flatpak and flatpak-builder (1.2+), configure the Flathub remote
through your normal system administration, then install the maintained 25.08
branch (choose `--arch=x86_64` or `--arch=aarch64` consistently if needed):

```sh
flatpak install --user flathub org.freedesktop.Platform//25.08 \
  org.freedesktop.Sdk//25.08 org.freedesktop.Sdk.Extension.rust-stable//25.08
python3 .github/scripts/channel_distribution.py flatpak \
  --allow-network --output /absolute/new-flatpak-stage
cd /absolute/new-flatpak-stage
flatpak-builder --user --force-clean build ad.yan.iceon.json
```

Preparation copies only allowlisted, tracked build inputs (plus shared Linux
metadata), rejects symlinks and excludes hidden files, `.git`, `.env`, credentials,
local config and `target`. `cargo vendor --locked --versioned-dirs` is the only
network-enabled preparation step. Omit `--allow-network` to require cached
packages. The generated `.cargo/config.toml` refers to relative `vendor` and
metadata is verified frozen/offline before exporting. Failed preparation leaves
no usable export. Dependency cache configuration is not copied. Run from a clean
reviewed checkout; source edits to tracked build files are intentionally included.
Never put credentials into source files. Cargo vendor may need all locked target
platform dependencies even when preparing on macOS.

Flatpak build commands always use `cargo build --frozen --offline`; there is no
build-time network permission. SDK provisioning/source downloads occur before
the isolated build. Branch selection is pinned, but SDK commits and the rolling
rust-stable extension must also be recorded/pinned externally for bit-for-bit
reproducibility. The staging directory can be archived and SHA256 recorded by CI;
it is not a credential-bearing copy of a user's home. Validate the package on
both architectures: native dependencies and GPUI GPU/windowing support have not
been established by these preparation files alone.

## Standalone remote manifest

Use the **official** flatpak-builder-tools cargo generator, not a custom crate
resolver. Fetch/review and pin a tools checkout outside this repository; install
its documented generator dependencies in an isolated environment:

```sh
python3 /reviewed/flatpak-builder-tools/cargo/flatpak-cargo-generator.py \
  Cargo.lock -o /absolute/cargo-sources.json
python3 .github/scripts/channel_distribution.py flatpak-remote \
  --revision FULL_40_HEX_COMMIT --source-archive /absolute/commit.tar.gz \
  --source-sha256 ACTUAL_ARCHIVE_SHA256 --cargo-sources /absolute/cargo-sources.json \
  --output /absolute/new-remote-flatpak-stage
cd /absolute/new-remote-flatpak-stage
flatpak-builder --user --force-clean build ad.yan.iceon.json
```

Acquire the actual archive from
`https://github.com/yan-ad/iceon/archive/FULL_40_HEX_COMMIT.tar.gz` and independently
verify its identity. Required commit/SHA inputs are never guessed; the command
checks the local archive bytes and emits the exact canonical immutable URL.
Use the Cargo.lock belonging to that same revision. Preparation inspects the
archive without extraction, rejects links/traversal, and requires generated crate
URLs/checksums to exactly match its Cargo.lock. Generated inline metadata still
requires review; this is not proof of remote archive provenance. Currently only
registry archive/inline sources are accepted; git dependencies require separate review.
Do not substitute a mutable branch/tag or an unverified source URL.

## Sandbox scope and functional gaps

Permissions are network (Kubernetes/Vault endpoints), GPU DRI, Wayland,
fallback X11 with shared IPC, and **exact** `org.freedesktop.secrets` access for
keyring storage. No home/host filesystem access, blanket session/system bus,
`org.freedesktop.Flatpak` talk permission, or `flatpak-spawn --host` is granted.
File chooser/document portals use Flatpak's default portal mediation; no blanket
portal bus permission is necessary. Desktop metadata deliberately does not
register `x-scheme-handler/iceon`: native activation is implemented only on macOS.

A selected kubeconfig may be exported through the document portal, but relative
certificate/key references, host absolute paths and sibling files do not gain
access automatically. Existing import preserves credential references rather
than copying secrets into the catalog, so portal grants/paths must remain valid
across reconnection. Self-contained configurations with embedded credentials
may work only after explicit chooser selection and must be tested. Neither the
manifest nor preparation rewrites references, copies credentials, or implements
new portal persistence/re-authorisation behavior. Do not relax permissions to
claim this is solved. GPUI's Linux file chooser/portal behavior still requires
native testing.

Host exec auth helpers (cloud CLIs, scripts), host kubeconfig discovery, host
shells and host-local terminal integrations are **not supported**. A terminal,
if explicitly started, runs only sandbox programs and is not a host terminal.
The current UI does not hide all unsupported actions. Consequently this is a
build/distribution preparation path, not full Flatpak feature parity. Secret
Service unlock, chooser cancellation, reconnect, missing helpers, Wayland/X11,
GPU rendering, and launch must be exercised before claiming support.

Official references verified during preparation:
- https://docs.flatpak.org/en/latest/available-runtimes.html (two-year Freedesktop support)
- https://github.com/flathub/org.freedesktop.Sdk.Extension.rust-stable/tree/branch/25.08
- https://github.com/flatpak/flatpak-builder-tools/blob/master/cargo/README.md
  (generator and supported vendored-tarball alternative)
- https://doc.rust-lang.org/cargo/commands/cargo-vendor.html
