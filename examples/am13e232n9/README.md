# Examples for the AM13E232N9 (LP-AM13E232 LaunchPad)

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
| `blinky` | Blinks USER_LED0 (red, PD0) and USER_LED1 (green, PD1). |
| `button` | Toggles USER_LED1 on every press of the wake/user button (PB0), using a GPIO edge interrupt. |
| `uart` | Blocking UART on the XDS110 backchannel: a greeting, the uptime, then an echo. |
| `uart_buffered` | Interrupt driven (buffered, async) UART echo, with a second task blinking USER_LED0. |
| `wwdt` | The window watchdog resetting the device, and the reset cause read back. |
| `selftest` | On-target checks of DMA, GPIO edge interrupts (port D) and the time driver. |

The LP-AM13E232 carries an AM13E232N9 in the 128-pin TQFP (`AM13E232N91TPDTR`), so the
`embassy-mspm0` feature is `am13e232n9pdt`. The pins are taken from the LaunchPad schematic in the
AM13E2x SDK 26.02.00.03 (`MCU205E1(001)_Sch.pdf`).

The UART examples use UC4 on PA0 (TX) and PA1 (RX), wired to the XDS110 backchannel UART. It shows
up on the host as the "XDS110 Class Application/User UART" port; open it at 115200 baud, 8N1.

**These examples build, but have not been run on an LP-AM13E232 yet.** The HAL they use is the one
tested on the LP-AM13E230 (`examples/am13e23019`): the AM13E232x has the same GPIO, DMA, timers,
UNICOMM and watchdog blocks, checked register by register against its SVD.

## Flashing

probe-rs does not support the AM13E2x yet, so `cargo run` flashes with DSLite through the
LaunchPad's XDS110, using `LP_AM13E232.ccxml` (see `flash.py` and `.cargo/config.toml`). DSLite comes
with [Code Composer Studio](https://www.ti.com/tool/CCSTUDIO) and
[UniFlash](https://www.ti.com/tool/UNIFLASH). `flash.py` looks for it in `C:/ti/ccs2101/ccs` (or
`~/ti/ccs2101/ccs`); set `CCS_ROOT` to the `ccs` directory of another installation, or `DSLITE` to the
executable. Close any debug session that holds the XDS110 first. Python 3 is needed.

After programming, `flash.py` issues a system reset, which starts the program.

### Flash ECC

The AM13E2x flash is programmed in 128-bit words protected by ECC, and a word can only be written
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

* The time driver counts LFCLK, from the internal 32.768 kHz LFOSC, which is only as accurate as
  that oscillator (+-3% on the AM13E230x).
* The parts run from SYSOSC at 32 MHz, the reset configuration. Clock configuration (SYSPLL, HFXT)
  is not implemented yet.
* The AM13E232x SDK gives 2 NVIC priority bits (the AM13E230x SDK 4), so the PAC uses 2.
* `defmt` over RTT needs a probe-rs supported probe, so the examples report over the UART instead.
