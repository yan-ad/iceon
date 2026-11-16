# Homebrew preparation only

No tap is cloned, modified remotely, or pushed. The intended empty tap is
`yan-ad/homebrew-tap`; publication requires separate authorization.

From the Iceon repository root (Python 3.11+), supply **real** files:

```sh
python3 .github/scripts/channel_distribution.py homebrew --version 0.1.5 \
  --checksums /absolute/release/SHA256SUMS --output /absolute/new-tap-export
# Or hash both actual DMGs directly:
python3 .github/scripts/channel_distribution.py homebrew --version 0.1.5 \
  --artifacts /absolute/release --output /absolute/new-tap-export
```

The input must cover `Iceon_0.1.5_aarch64.dmg` and
`Iceon_0.1.5_x86_64.dmg`. Checksums use `sha256sum` format (hex, two spaces or
space/star, basename). Duplicate names, malformed hashes, missing architectures,
version mismatch, and existing export directories fail closed. Both options can
be combined to verify artifacts against the supplied checksum file. A checksum
file is an explicit trust input: preparation cannot prove it was produced by the
release pipeline or that its matching GitHub assets have been published.

Only after validation does the export contain `README.md` and `Casks/iceon.rb`.
No placeholder cask or `sha256 :no_check` is checked in. Review the exported diff,
run Ruby/Homebrew checks on macOS and confirm release URLs, signing and
notarization before separately authorizing any tap publication.
