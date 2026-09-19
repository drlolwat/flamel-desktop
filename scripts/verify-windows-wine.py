"""Exercise the actual NSIS payload in an isolated Wine prefix, not WebView2.

A test-only registry fixture represents
an existing WebView2 installation so this checks NSIS install/uninstall and all
MCP executables without downloading a browser into Wine. The release installer
itself is unmodified. Browser/UI validation is separate and must use an actual WebView2 runtime.
"""
import json
import os
from pathlib import Path
import selectors
import signal
import subprocess
import sys
import tempfile
import time

SIDECARS = ("flamel_mcp", "git_mcp", "docs_mcp", "db_mcp", "browser_mcp", "memory_mcp")


def wine(*args, **kwargs):
    return subprocess.run(
        ["xvfb-run", "-a", "wine", *map(str, args)],
        check=True, timeout=180, **kwargs,
    )


def initialize_mcp(command):
    request = json.dumps({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
        "protocolVersion": "2024-11-05", "capabilities": {},
        "clientInfo": {"name": "flamel-release-check", "version": "1"},
    }}) + "\n"
    # Keep stdin open until the response. flamel_mcp dispatches on threads and
    # closing stdin immediately can terminate it before that thread replies.
    with tempfile.TemporaryFile() as errors, selectors.DefaultSelector() as selector:
        process = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                   stderr=errors, text=True, start_new_session=True)
        try:
            process.stdin.write(request)
            process.stdin.flush()
            selector.register(process.stdout, selectors.EVENT_READ)
            deadline = time.monotonic() + 90
            while time.monotonic() < deadline:
                if not selector.select(max(0, deadline - time.monotonic())):
                    break
                line = process.stdout.readline()
                if not line:
                    break
                if line.startswith("{"):
                    message = json.loads(line)
                    if message.get("id") == 1:
                        if "result" not in message:
                            raise RuntimeError(f"MCP initialize returned an error: {message}")
                        process.stdin.close()
                        if process.wait(timeout=15) != 0:
                            raise RuntimeError("MCP failed after initialization")
                        return
            raise RuntimeError("Installed MCP did not initialize under Wine")
        finally:
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait()
            process.stdout.close()
            if not process.stdin.closed:
                process.stdin.close()


def main(installer):
    prefix = Path(os.environ["WINEPREFIX"])
    if not prefix.is_absolute() or not prefix.name.startswith("flamel-wine."):
        raise RuntimeError("Installer validation requires a disposable Flamel Wine prefix")
    registry = r"HKCU\Software\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}"
    os.environ.setdefault("WINEDEBUG", "-all")
    wine("reg", "add", registry, "/v", "pv", "/t", "REG_SZ", "/d", "999.0.0.0", "/f")
    wine(Path(installer).resolve(), "/S", r"/D=C:\FlamelReleaseTest")
    installed = prefix / "drive_c/FlamelReleaseTest"
    if not (installed / "flamel.exe").is_file():
        raise RuntimeError("NSIS exited without installing flamel.exe")
    entries = list((installed / "resources/bin").iterdir())
    if any(p.is_symlink() or not p.is_file() for p in entries):
        raise RuntimeError("Unexpected non-file tool payload")
    actual = {p.name for p in entries}
    expected = {name + ".exe" for name in SIDECARS}
    if actual != expected:
        raise RuntimeError(f"Unexpected installer sidecars: {actual ^ expected}")
    for required in ("LICENSE.md", "NOTICE", "THIRD_PARTY_NOTICES.md"):
        if not (installed / "resources/notices" / required).is_file():
            raise RuntimeError(f"Installer is missing {required}")
    for name in SIDECARS:
        executable = installed / "resources/bin" / f"{name}.exe"
        if not executable.is_file():
            raise RuntimeError(f"Installer is missing {name}.exe")
        initialize_mcp(["xvfb-run", "-a", "wine", str(executable)])
        print(f"Installed {name}: initialize passed", flush=True)
    wine(installed / "uninstall.exe", "/S", f"_?=C:\\FlamelReleaseTest")
    if (installed / "flamel.exe").exists():
        raise RuntimeError("NSIS uninstall left the application executable installed")
    print("Wine NSIS install/uninstall and six installed MCP initializations passed.")
    print("WebView2 fixture used: this does not certify native Windows GUI/browser behavior.")


if __name__ == "__main__":
    main(sys.argv[1])
