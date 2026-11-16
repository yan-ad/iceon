"""Offline contracts for the build-only distribution workflow."""
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[2]


class DistributionWorkflowTests(unittest.TestCase):
    def test_manual_only_read_only_no_release_or_tap_mutation(self):
        text = (ROOT / '.github/workflows/distribution.yml').read_text()
        self.assertIn('workflow_dispatch:', text)
        self.assertIn('contents: read', text)
        self.assertNotIn('contents: write', text)
        for forbidden in ['gh release', 'git push', 'TAURI_SIGNING', 'native_release.py', 'pull_request_target:']:
            self.assertNotIn(forbidden, text)
        self.assertNotIn('\n  push:', text)

    def test_all_native_architectures_and_opt_in_sandbox(self):
        text = (ROOT / '.github/workflows/distribution.yml').read_text()
        for target in ['x86_64-unknown-linux-gnu', 'aarch64-unknown-linux-gnu',
                       'aarch64-apple-darwin', 'x86_64-apple-darwin',
                       'i686-pc-windows-msvc', 'aarch64-pc-windows-msvc']:
            self.assertIn('target: ' + target, text)
        self.assertIn('if: inputs.build_flatpak', text)
        self.assertIn('default: false', text)
        self.assertIn('homebrew-tap-export-NOT-PUBLISHED', text)
        self.assertIn('channel_distribution.py homebrew', text)
        self.assertIn('flatpak build-bundle', text)


if __name__ == '__main__':
    unittest.main()
