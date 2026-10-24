#!/usr/bin/env python3
"""Hardware-in-the-loop check of the examples on an LP-AM13E232.

Flashes each example with `cargo run --release` (which uses flash.py) and checks what it prints on
the XDS110 backchannel UART. Needs pyserial.

    python hil_test.py [COM port]    (default COM6, or /dev/ttyACM0 on Linux)
"""

import os
import re
import shutil
import subprocess
import sys
import threading
import time

import serial

PORT = sys.argv[1] if len(sys.argv) > 1 else ("COM6" if os.name == "nt" else "/dev/ttyACM0")


class Capture:
    """Collects everything the board sends while an example is flashed and run."""

    def __init__(self, port):
        self.port = port
        self.data = bytearray()
        self.stop = threading.Event()
        self.thread = threading.Thread(target=self.run, daemon=True)
        self.thread.start()

    def run(self):
        while not self.stop.is_set():
            self.data += self.port.read(256)

    def text(self):
        return self.data.decode(errors="replace")

    def close(self):
        self.stop.set()
        self.thread.join()


CHIP = "AM13E232N9"
ELF = os.path.join("target", "thumbv8m.main-none-eabihf", "release")


def flash(example):
    """Build an example, program it and start it: with probe-rs if it is installed (it keeps
    `probe-rs run` attached, so download and reset are run separately), otherwise with DSLite."""
    quiet = dict(check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    subprocess.run(["cargo", "build", "--release", "--bin", example], **quiet)
    elf = os.path.join(ELF, example)
    if shutil.which("probe-rs"):
        subprocess.run(["probe-rs", "download", "--chip", CHIP, elf], **quiet)
        subprocess.run(["probe-rs", "reset", "--chip", CHIP], **quiet)
    else:
        subprocess.run([sys.executable, "flash.py", elf], **quiet)


def wait_for(capture, pattern, timeout):
    deadline = time.time() + timeout
    while time.time() < deadline:
        m = re.search(pattern, capture.text())
        if m:
            return m
        time.sleep(0.05)
    return None


def test_uart(port):
    capture = Capture(port)
    flash("uart")
    ok = wait_for(capture, r"echoing\r\n", 15) is not None
    capture.close()
    out = capture.text()

    # DSLite resets the device when it connects, so the image that was on it before may print
    # first. Only the output after the last greeting is from the new image.
    start = out.rfind("Hello Embassy World from the AM13E232!\r\n")
    greeting = start >= 0
    if greeting:
        out = out[start:]
    uptimes = [int(t) for t in re.findall(r"uptime: (\d+) ms", out)]
    # The three readings are taken one second apart by the time driver (TIMG4 on LFCLK).
    steps = [b - a for a, b in zip(uptimes, uptimes[1:])]
    timing = len(steps) == 2 and all(995 <= s <= 1010 for s in steps)

    port.reset_input_buffer()
    msg = b"The quick brown fox jumps over the lazy dog 0123456789\r\n"
    port.write(msg)
    time.sleep(0.5)
    echo = port.read(len(msg) * 2) == msg

    print(f"uart: greeting={greeting} uptimes={uptimes} timing={timing} echo={echo}")
    return ok and greeting and timing and echo


def test_uart_buffered(port):
    capture = Capture(port)
    flash("uart_buffered")
    ok = wait_for(capture, r"Hello Embassy World \(buffered\)!\r\n", 15) is not None
    capture.close()

    # A block bigger than the 32 byte buffers, echoed through the interrupt driven driver.
    port.reset_input_buffer()
    msg = bytes(range(0x20, 0x7F)) * 4
    port.write(msg)
    time.sleep(1.0)
    got = port.read(len(msg) * 2)
    echo = got == msg

    print(f"uart_buffered: greeting={ok} echo={echo} ({len(got)}/{len(msg)} bytes)")
    return ok and echo


def test_selftest(port):
    capture = Capture(port)
    flash("selftest")
    m = wait_for(capture, r"selftest: (\d+) passed, (\d+) failed\r\n", 20)
    capture.close()

    for line in capture.text().splitlines():
        if line:
            print(f"  {line}")

    return m is not None and int(m.group(1)) > 0 and int(m.group(2)) == 0


def test_wwdt(port):
    capture = Capture(port)
    flash("wwdt")
    # The watchdog resets the device about a second after the last pet; the example then reports
    # why it was reset.
    m = wait_for(capture, r"reset cause: Ok\(BootrstWwdt0Violation\)", 15)
    capture.close()

    pets = capture.text().count("pet\r\n")
    print(f"wwdt: pets={pets} watchdog_reset={m is not None}")
    return m is not None and pets >= 5


def test_ethercat(port):
    capture = Capture(port)
    flash("ethercat")
    m = wait_for(capture, r"ethercat: (\d) of 2 PHYs found\r\n", 15)
    capture.close()

    for line in capture.text().splitlines():
        if line.startswith("PHY"):
            print(f"  {line}")
    print(f"ethercat: {m.group(0).strip() if m else 'no result'}")
    return m is not None and m.group(1) == "2"


def main():
    with serial.Serial(PORT, 115200, timeout=0.1) as port:
        results = {
            "uart": test_uart(port),
            "uart_buffered": test_uart_buffered(port),
            "selftest": test_selftest(port),
            "wwdt": test_wwdt(port),
            "ethercat": test_ethercat(port),
        }

    for name, passed in results.items():
        print(f"{'PASS' if passed else 'FAIL'} {name}")

    sys.exit(0 if all(results.values()) else 1)


if __name__ == "__main__":
    main()
