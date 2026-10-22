# Examples for the AM13E23019 (LP-AM13E230 LaunchPad)

Run individual examples with
```
cargo run --release --bin <module-name>
```
for example
```
cargo run --release --bin uart
```

| Example | What it shows |
|---|---|
| `blinky` | Blinks LED4 (red, PA5) and LED5 (green, PB0). |
| `button` | Toggles LED5 on every press of S2 (PB13), using a GPIO edge interrupt. |
| `uart` | Blocking UART on the XDS110 backchannel: a greeting, the uptime, then an echo. |
| `uart_buffered` | Interrupt driven (buffered, async) UART echo, with a second task blinking LED4. |
| `wwdt` | The window watchdog resetting the device, and the reset cause read back. |
| `selftest` | On-target checks of DMA, GPIO edge interrupts and the time driver. |

The UART examples use UC4 on PA0 (TX) and PA1 (RX), wired to the XDS110 backchannel UART. It shows
up on the host as the "XDS110 Class Application/User UART" port; open it at 115200 baud, 8N1.
(Rev E1 boards need the jumper wires described in the LaunchPad user's guide for this to work.)

## Flashing

probe-rs does not support the AM13E230x yet, so `cargo run` flashes with DSLite through the
LaunchPad's XDS110 (see `flash.py` and `.cargo/config.toml`). DSLite comes with
[Code Composer Studio](https://www.ti.com/tool/CCSTUDIO) and [UniFlash](https://www.ti.com/tool/UNIFLASH).
`flash.py` looks for it in `C:/ti/ccs2101/ccs` (or `~/ti/ccs2101/ccs`); set `CCS_ROOT` to the
`ccs` directory of another installation, or `DSLITE` to the executable. Close any debug session that
holds the XDS110 first. Python 3 is needed.

After programming, `flash.py` issues a system reset, which starts the program.

### Flash ECC

The AM13E230x flash is programmed in 128-bit words protected by ECC, and a word can only be written
once between erases. `memory.x` therefore starts every flash section of cortex-m-rt's `link.x` on a
16 byte boundary, so that no two segments of the ELF share a flash word. Keep that part of `memory.x`
in your own projects.

## Hardware in the loop

`hil_test.py` flashes `uart`, `uart_buffered`, `selftest` and `wwdt` one after the other and checks
their output on the backchannel UART. It needs pyserial (`pip install pyserial`).
```
python hil_test.py COM6
```

## Notes

* The time driver counts LFCLK, from the internal 32.768 kHz LFOSC, which is only accurate to +-3%
  (+-5% above 85 °C). On the board used to test this, it ran 3% slow: `Timer::after_secs(1)` took
  1.03 s. SYSOSC, which clocks the CPU and the UARTs, is accurate to +-1%.
* The parts run from SYSOSC at 32 MHz, the reset configuration. Clock configuration (SYSPLL, HFXT)
  is not implemented yet.
* `defmt` over RTT needs a probe-rs supported probe, so the examples report over the UART instead.
