#!/usr/bin/env python3
"""Cargo runner: flash an ELF to the LP-AM13E230 with DSLite and start it.

probe-rs has no AM13E230x support yet. DSLite ships with Code Composer Studio and UniFlash and
drives the LaunchPad's XDS110 with TI's flash loader, which also knows the ECC rules of the
AM13 flash.

Set CCS_ROOT to the `ccs` directory of your CCS installation (default C:/ti/ccs2101/ccs, or
~/ti/ccs2101/ccs on Linux and macOS), or DSLITE to the DSLite executable itself.
"""

import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path


def find_dslite() -> Path:
    if "DSLITE" in os.environ:
        return Path(os.environ["DSLITE"])

    exe = "DSLite.exe" if os.name == "nt" else "DSLite"
    roots = []
    if "CCS_ROOT" in os.environ:
        roots.append(Path(os.environ["CCS_ROOT"]))
    roots += [Path("C:/ti/ccs2101/ccs"), Path.home() / "ti" / "ccs2101" / "ccs"]

    for root in roots:
        candidate = root / "ccs_base" / "DebugServer" / "bin" / exe
        if candidate.exists():
            return candidate

    found = shutil.which("DSLite") or shutil.which("dslite")
    if found:
        return Path(found)

    sys.exit("flash.py: DSLite not found. Set CCS_ROOT or DSLITE.")


def main() -> None:
    if len(sys.argv) < 2:
        sys.exit("usage: flash.py <elf>")

    elf = Path(sys.argv[1])
    ccxml = Path(__file__).resolve().parent / "LP_AM13E230.ccxml"

    # DSLite picks the loader by file extension; cargo's binaries have none.
    with tempfile.TemporaryDirectory() as tmp:
        image = Path(tmp) / f"{elf.name}.out"
        shutil.copyfile(elf, image)

        # Program and verify, then apply a system reset (reset operation 1, see `DSLite flash
        # -c LP_AM13E230.ccxml -R`), which starts the program from a clean device state. Running
        # from the entry point right after loading (`-u`) leaves the device as the flash loader
        # left it, and the program does not start reliably.
        cmd = [
            str(find_dslite()), "flash", "-c", str(ccxml),
            "-e", "-f", "-v", "-r", "1",
            str(image),
        ]
        print(" ".join(cmd), flush=True)
        sys.exit(subprocess.call(cmd))


if __name__ == "__main__":
    main()
