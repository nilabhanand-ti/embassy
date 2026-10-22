# Embassy MSPM0 HAL

The embassy-mspm0 HAL aims to provide a safe, idiomatic hardware abstraction layer for all MSPM0 and MSPS003 chips, and for the AM13E230x and AM13E232x.

* [Documentation](https://docs.embassy.dev/embassy-mspm0/) (**Important:** use docs.embassy.dev rather than docs.rs to see the specific docs for the chip you’re using!)
* [Source](https://github.com/embassy-rs/embassy/tree/main/embassy-mspm0)
* [Examples](https://github.com/embassy-rs/embassy/tree/main/examples)

## AM13E2x (AM13E230x, AM13E232x)

The AM13E230x and AM13E232x (Arm Cortex-M33) are built from the same GPIO, IOMUX, DMA, timer and
UNICOMM peripherals as the MSPM0 parts, and are supported by this HAL with the `am13e230*` and
`am13e232*` chip features: `am13e23019gpm` for the LP-AM13E230 LaunchPad and `am13e232n9pdt` for
the LP-AM13E232. Build for `thumbv8m.main-none-eabihf`. The drivers check the `am13` cfg, which
both families set.

Tested on the LP-AM13E230: GPIO (including edge interrupts), the UNICOMM instances as UARTs
(blocking and buffered/async), DMA, the time driver (TIMG4 on LFCLK), the window watchdog and the
reset cause. The AM13E232x has the same blocks (compared register by register) and builds, but has
not been run on hardware yet. CLK_OUT builds but is untested. The ADC, I2C and SPI of the UNICOMM
instances, timer PWM, the AM13E232x EtherCAT, ePWM, SDFM and FSI, low power modes and clock
configuration (the parts run from SYSOSC at 32 MHz, the reset configuration) are not implemented
yet.

The time driver counts LFCLK, which runs from the internal LFOSC (32.768 kHz, +-3% on the
AM13E230x), so time is only as accurate as LFOSC. Use the executor of `embassy-executor`
(`platform-cortex-m`), not the `executor-*` features of this crate, which need `low-power`.

See [examples/am13e23019](https://github.com/nilabhanand-ti/embassy/tree/am13e2/examples/am13e23019)
and [examples/am13e232n9](https://github.com/nilabhanand-ti/embassy/tree/am13e2/examples/am13e232n9).

## Embedded-hal

The `embassy-mspm0` HAL implements the traits from [embedded-hal](https://crates.io/crates/embedded-hal) (1.0) and [embedded-hal-async](https://crates.io/crates/embedded-hal-async), as well as [embedded-io](https://crates.io/crates/embedded-io) and [embedded-io-async](https://crates.io/crates/embedded-io-async).

## A note on feature flag names

Feature flag names for chips do not include temperature rating or distribution format.

Usually chapter 10 of your device's datasheet will explain the device nomenclature and how to decode it. Feature names in embassy-mspm0 only use the following from device nomenclature:
- MCU platform
- Product family
- Device subfamily
- Flash memory
- Package type

This means for a part such as `MSPM0G3507SPMR`, the feature name is `mspm0g3507pm`. This also means that `MSPM0G3507QPMRQ1` uses the feature `mspm0g3507pm`, since the Q1 parts are just qualified variants of the base G3507 with a PM (QFP-64) package.

## Interoperability

This crate can run on any executor.
