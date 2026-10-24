# Embassy AM13 HAL

The embassy-am13 HAL aims to provide a safe, idiomatic hardware abstraction layer for the Texas
Instruments AM13E2x microcontrollers: the AM13E230x and the AM13E232x (Arm Cortex-M33 with FPU,
`thumbv8m.main-none-eabihf`).

* [Documentation](https://docs.embassy.dev/embassy-am13/) (**Important:** use docs.embassy.dev rather than docs.rs to see the specific docs for the chip you’re using!)
* [Source](https://github.com/embassy-rs/embassy/tree/main/embassy-am13)
* [Examples](https://github.com/embassy-rs/embassy/tree/main/examples): `am13e23019` (LP-AM13E230)
  and `am13e232n9` (LP-AM13E232)

Select the chip with its feature: `am13e23019gpm` for the LP-AM13E230 LaunchPad, `am13e232n9pdt`
for the LP-AM13E232. The chip definitions come from
[mspm0-data](https://github.com/mspm0-rs/mspm0-data), where the AM13E2x are generated alongside
the MSPM0 parts.

## Peripherals

| Peripheral | Driver | Notes |
|---|---|---|
| GPIO | `gpio` | All ports, edge interrupts |
| DMA | `dma` | 12 channels |
| UART | `uart` | UNICOMM instances in UART mode: blocking and buffered (async) |
| SPI | `spi` | UNICOMM instances with SPI (UC0, UC1, UC3, UC4): controller, blocking and async, 8 and 16-bit words, internal loopback |
| I2C | `i2c` | UNICOMM instances with burst support (UC2, UC5): controller, blocking and async, up to 4095 bytes per transfer |
| ADC | `adc` | ADC0 to ADC2, single conversions on pins, VREFLO, the internal test channel and the temperature sensor; blocking and async |
| Timers | `tim` | PWM on TIMG4 (16-bit) and TIMG12 (32-bit); TIMG4 runs the time driver |
| Watchdog | `wwdt` | WWDT0 |
| SYSCTL | `sysctl` | CLK_OUT, reset cause |

Not implemented yet: clock configuration (the parts run from SYSOSC at 32 MHz, the reset
configuration), low power modes, the UNICOMM I2C target, DMA for the serial drivers, CAN (MCAN),
the motor control PWMs (MCPWM, ePWM), CMPSS, PGA, ECAP, EQEP, and on the AM13E232x the EtherCAT
slave controller, SDFM and FSI.

The time driver counts LFCLK, which runs from the internal LFOSC (32.768 kHz, ±3%), so time is only
as accurate as LFOSC.

## Flash ECC

The AM13E2x flash is programmed in 128-bit words protected by ECC, and a word can only be written
once between erases. Start every flash section of the linker script on a 16 byte boundary so that no
two segments of the ELF share a flash word; see `memory.x` in the examples.
