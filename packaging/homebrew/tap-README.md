# Iceon Homebrew tap (prepared locally)

Intended repository: https://github.com/yan-ad/homebrew-tap

The generated `Casks/iceon.rb` is version-pinned with separate Apple Silicon and
Intel SHA256 values from release artifacts/checksums. Downloads always come from
https://github.com/yan-ad/iceon/releases, not this tap.

After an explicit, separately authorized publication and release verification:

```sh
brew tap yan-ad/tap
brew install --cask yan-ad/tap/iceon
```

Local review (does not publish): `brew style Casks/iceon.rb` and
`brew audit --cask Casks/iceon.rb`. Installation downloads the canonical release;
do not run it until that exact release exists. Preparation does not establish
Developer ID signing, notarization, Gatekeeper acceptance or Homebrew acceptance.
No quarantine bypass, auto-update declaration or user-data deletion is included.
