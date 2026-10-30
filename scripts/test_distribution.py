"""Distribution architecture guards, without downloads or release builds.

Run: uv run --no-project python -m unittest discover -s scripts -p test_distribution.py
"""

import os
import shutil
import tarfile
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parent.parent


class DistributionTests(unittest.TestCase):
    def setUp(self):
        self.work = tempfile.TemporaryDirectory(prefix="herdr-distribution-")
        self.addCleanup(self.work.cleanup)
        self.bin = Path(self.work.name)
        self.stub("uname", 'case "$1" in -s) echo "$TEST_OS";; -m) echo "$TEST_ARCH";; esac')
        self.stub("curl", 'echo "unexpected download" >&2; exit 97')
        self.stub("cargo", 'echo "unexpected build" >&2; exit 97')
        self.stub("sysctl", 'exit 1')

    def stub(self, name, body):
        path = self.bin / name
        path.write_text("#!/bin/sh\n" + body + "\n")
        path.chmod(0o755)

    def run_script(self, script, os_name, arch, *args):
        env = dict(os.environ, PATH=f"{self.bin}:{os.environ['PATH']}", TEST_OS=os_name, TEST_ARCH=arch)
        return subprocess.run(
            [str(ROOT / script), *args], cwd=ROOT, env=env,
            capture_output=True, text=True, check=False,
        )

    def test_intel_mac_installer_stops_before_download(self):
        for arch in ("x86_64", "amd64"):
            with self.subTest(arch=arch):
                result = self.run_script("distribution/install.sh", "Darwin", arch)
                self.assertEqual(result.returncode, 1)
                self.assertIn("Intel Macs aren't supported; build from source", result.stderr)
                self.assertNotIn("unexpected download", result.stderr)

    def test_rosetta_mac_installer_uses_arm64(self):
        for key in ("hw.optional.arm64", "sysctl.proc_translated"):
            with self.subTest(key=key):
                self.stub("sysctl", f'[ "$2" = "{key}" ] && echo 1')
                self.stub("curl", '''while [ "$#" -gt 0 ]; do
  if [ "$1" = -o ]; then
    printf '%s' '{"version":"test","macos-aarch64":{"url":"test","sha256":"test"}}' > "$2"
    exit 0
  fi
  shift
done
exit 97''')
                result = self.run_script("distribution/install.sh", "Darwin", "x86_64")
                self.assertEqual(result.returncode, 1)
                self.assertIn("Installing Herdr test for macos-aarch64", result.stdout)
                self.assertIn("download is damaged", result.stderr)
                self.assertNotIn("Intel Macs", result.stderr)

    def download_url(self, url, releases="https://releases.example", installed=False):
        """The URL install.sh downloads the app from, for a manifest giving `url`."""
        home = Path(self.work.name) / "home"
        data = home / ".local/share"
        shutil.rmtree(data / "herdr-app", ignore_errors=True)
        (data / "herdr-app" if installed else data).mkdir(parents=True, exist_ok=True)
        log = Path(self.work.name) / "urls"
        log.unlink(missing_ok=True)
        manifest = Path(self.work.name) / "latest.json"
        manifest.write_text('{"version":"1.0.0","targets":{"linux-x86_64":{"url":"%s","sha256":"x"}}}' % url)
        self.stub("curl", f'''out=; url=; prev=
for a; do
  if [ "$prev" = -o ]; then out=$a; else case $a in -*) ;; *) url=$a ;; esac; fi
  prev=$a
done
echo "$url" >> "{log}"
case $url in */latest.json) cp "{manifest}" "$out"; exit 0 ;; esac
exit 22''')
        env = dict(os.environ, PATH=f"{self.bin}:{os.environ['PATH']}", TEST_OS="Linux", TEST_ARCH="x86_64",
                   HOME=str(home), XDG_DATA_HOME=str(data), HERDR_RELEASES_URL=releases)
        result = subprocess.run([str(ROOT / "distribution/install.sh")], cwd=ROOT, env=env,
                                capture_output=True, text=True, check=False)
        self.assertIn("download failed", result.stderr)
        return log.read_text().splitlines()[-1]

    def test_installer_marks_plain_release_downloads_for_the_counter(self):
        r = "https://releases.example"
        tarball = f"{r}/app/1.0.0/herdr-app-linux-x86_64.tar.gz"
        self.assertEqual(self.download_url(tarball), f"{tarball}?src=install")
        self.assertEqual(self.download_url(tarball, installed=True), f"{tarball}?src=reinstall")
        # A mirror's query (a signed URL) or fragment, or another host: exactly as given.
        for url in (f"{tarball}?sig=a%2Fb&exp=1", f"{tarball}?src=update", f"{tarball}#x",
                    "https://mirror.example/app/1.0.0/herdr-app-linux-x86_64.tar.gz",
                    "https://releases.example.evil/app/1.0.0/herdr-app-linux-x86_64.tar.gz"):
            with self.subTest(url=url):
                self.assertEqual(self.download_url(url), url)
        # A test server elsewhere marks its own URLs.
        local = "http://127.0.0.1:8000/e2e"
        self.assertEqual(self.download_url(f"{local}/app/1.0.0/a.tar.gz", releases=local), f"{local}/app/1.0.0/a.tar.gz?src=install")

    def test_intel_mac_package_stops_before_build(self):
        for args in ((), ("x86_64-apple-darwin",)):
            with self.subTest(args=args):
                result = self.run_script("scripts/package", "Darwin", "x86_64", *args)
                self.assertEqual(result.returncode, 1)
                self.assertIn("Intel Macs aren't supported; build from source", result.stderr)
                self.assertNotIn("unexpected build", result.stderr)

    def test_supported_installers_still_try_download(self):
        for os_name, arch in (("Darwin", "arm64"), ("Linux", "x86_64"), ("Linux", "aarch64")):
            with self.subTest(os=os_name, arch=arch):
                result = self.run_script("distribution/install.sh", os_name, arch)
                self.assertEqual(result.returncode, 1)
                self.assertIn("unexpected download", result.stderr)
                self.assertNotIn("Intel Macs", result.stderr)

    def test_packages_keep_full_notices_on_every_platform(self):
        checkout = self.bin / "checkout"
        for path in [
            "Cargo.toml", "LICENSE", "THIRD-PARTY-NOTICES", "scripts/package",
            "assets/icons/herdr-ui-icon-clean.svg", "assets/icons/Herdr.icns",
        ]:
            destination = checkout / path
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(ROOT / path, destination)
        sign = checkout / "scripts/mac-sign"
        sign.write_text("#!/bin/sh\nexit 0\n")
        sign.chmod(0o755)
        release = checkout / "target/release"
        release.mkdir(parents=True)
        for name in ("herdr-app", "herdr-app.exe"):
            (release / name).write_bytes(b"test binary")
        self.stub("cargo", "exit 0")
        for os_name, arch, target, prefix in (
            ("Linux", "x86_64", "linux-x86_64", "herdr-app"),
            ("Darwin", "arm64", "macos-aarch64", "Herdr.app/Contents/Resources"),
            ("MINGW64_NT", "x86_64", "windows-x86_64", "herdr-app"),
        ):
            with self.subTest(target=target):
                result = self.run_script(checkout / "scripts/package", os_name, arch)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                with tarfile.open(checkout / f"dist/herdr-app-{target}.tar.gz") as archive:
                    for name in ("LICENSE", "THIRD-PARTY-NOTICES"):
                        self.assertEqual(
                            archive.extractfile(f"{prefix}/{name}").read(),
                            (ROOT / name).read_bytes(),
                        )
        self.stub("cargo", 'echo "unexpected build" >&2; exit 97')
        for name in ("LICENSE", "THIRD-PARTY-NOTICES"):
            notice = checkout / name
            original = notice.read_bytes()
            for state in ("empty", "missing"):
                with self.subTest(notice=name, state=state):
                    if state == "empty":
                        notice.write_bytes(b"")
                    else:
                        notice.unlink()
                    result = self.run_script(checkout / "scripts/package", "Linux", "x86_64")
                    self.assertEqual(result.returncode, 1)
                    self.assertIn(f"{name} is missing or empty", result.stderr)
                    self.assertNotIn("unexpected build", result.stderr)
            notice.write_bytes(original)

    def test_supported_packages_still_try_build(self):
        for os_name, arch in (("Darwin", "arm64"), ("Linux", "x86_64"), ("Linux", "aarch64")):
            with self.subTest(os=os_name, arch=arch):
                result = self.run_script("scripts/package", os_name, arch)
                self.assertEqual(result.returncode, 97)
                self.assertIn("unexpected build", result.stderr)
                self.assertNotIn("Intel Macs", result.stderr)


if __name__ == "__main__":
    unittest.main()
