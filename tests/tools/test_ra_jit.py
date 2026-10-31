"""Verify the installer boundary and isolation of experimental build settings."""
import hashlib
import importlib.machinery
import importlib.util
import io
from pathlib import Path
import tarfile
import tempfile
import unittest
from unittest.mock import patch

loader = importlib.machinery.SourceFileLoader("ra_jit_tool", str(Path(__file__).resolve().parents[2] / "tools/ra-jit"))
spec = importlib.util.spec_from_loader(loader.name, loader)
tool = importlib.util.module_from_spec(spec)
loader.exec_module(tool)
TRIPLE = "x86_64-unknown-linux-gnu"


def archive(link=False):
    data = io.BytesIO()
    with tarfile.open(fileobj=data, mode="w:gz") as bundle:
        for name in (*tool.PROGRAMS, "LICENSE-MIT", "LICENSE-APACHE", "README.md"):
            member = tarfile.TarInfo(f"ra-jit-{TRIPLE}/{name}")
            if link and name == "ra-jit":
                member.type = tarfile.SYMTYPE
                member.linkname = "/tmp/not-an-owned-executable"
                bundle.addfile(member)
            else:
                member.size = 7
                bundle.addfile(member, io.BytesIO(b"fixture"))
    return data.getvalue()


class InstallerTests(unittest.TestCase):
    def test_checksum_and_regular_file_checks_precede_installation(self):
        for wrong_digest, link in [(True, False), (False, True), (False, False)]:
            with self.subTest(wrong_digest=wrong_digest, link=link), tempfile.TemporaryDirectory() as temporary:
                data = archive(link)
                digest = "0" * 64 if wrong_digest else hashlib.sha256(data).hexdigest()
                pin = {**tool.PIN, "archives": {TRIPLE: digest}}
                destination = Path(temporary) / "installation"
                with patch.object(tool, "PIN", pin), patch.object(tool.urllib.request, "urlopen", return_value=io.BytesIO(data)):
                    if wrong_digest or link:
                        with self.assertRaises(RuntimeError):
                            tool.install(destination, TRIPLE)
                        self.assertFalse(destination.exists())
                    else:
                        tool.install(destination, TRIPLE)
                        self.assertEqual((destination / "ra-jit").read_bytes(), b"fixture")
                        self.assertEqual((destination / "ra-jit").stat().st_mode & 0o777, 0o755)
                        with patch.object(tool.urllib.request, "urlopen", side_effect=AssertionError("unexpected update")):
                            tool.install(destination, TRIPLE)

    def test_run_scopes_compiler_and_output_and_forwards_program_arguments(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            installed = root / "target/tools/ra-jit" / tool.PIN["archives"][TRIPLE] / TRIPLE
            installed.mkdir(parents=True)
            for program in tool.PROGRAMS:
                (installed / program).touch()
            rustc = "/toolchains/1.99.0/bin/rustc"
            with patch.object(tool, "ROOT", root), patch.object(tool, "target", return_value=TRIPLE), \
                    patch.object(tool.sys, "argv", ["ra-jit", "run", "--", "--release"]), \
                    patch.object(tool.subprocess, "check_output", return_value=rustc + "\n"), \
                    patch.object(tool, "daemon_directory", return_value=root / "daemon"), \
                    patch.object(tool.os, "chdir") as chdir, patch.object(tool.os, "execve") as execute, \
                    patch.dict(tool.os.environ, {"RUSTUP_TOOLCHAIN": "1.99.1", "CARGO_TARGET_DIR": "/normal-target"}, clear=True):
                tool.main()
                program, arguments, environment = execute.call_args.args
                self.assertEqual(program, Path("/toolchains/1.99.0/bin/cargo"))
                self.assertEqual(arguments, ["cargo", "ra-jit", "run", "--", "--release"])
                self.assertEqual(environment["RUSTUP_TOOLCHAIN"], "1.99.0")
                self.assertEqual(environment["RUSTC"], rustc)
                self.assertEqual(environment["CARGO_PROFILE_DEV_DEBUG"], "0")
                self.assertEqual(environment["CARGO_PROFILE_TEST_DEBUG"], "0")
                self.assertEqual(environment["CARGO_TARGET_DIR"], str(root / "target/ra-jit-build"))
                self.assertEqual(tool.os.environ["RUSTUP_TOOLCHAIN"], "1.99.1")
                self.assertEqual(tool.os.environ["CARGO_TARGET_DIR"], "/normal-target")
                chdir.assert_called_once_with(root)

    def test_release_and_shared_output_are_not_selected_by_accident(self):
        for arguments in [["build", "--release"], ["build", "--target-dir=/normal-target"], ["test", "--profile", "release"]]:
            with self.subTest(arguments=arguments), patch.object(tool.sys, "argv", ["ra-jit", *arguments]):
                with self.assertRaises(RuntimeError):
                    tool.main()


if __name__ == "__main__":
    unittest.main()
