# Embassy MSPM0 HAL

The embassy-mspm0 HAL aims to provide a safe, idiomatic hardware abstraction layer for all MSPM0 and MSPS003 chips, and for the AM13E230x.

* [Documentation](https://docs.embassy.dev/embassy-mspm0/) (**Important:** use docs.embassy.dev rather than docs.rs to see the specific docs for the chip you’re using!)
* [Source](https://github.com/embassy-rs/embassy/tree/main/embassy-mspm0)
* [Examples](https://github.com/embassy-rs/embassy/tree/main/examples)

## AM13E230x

The AM13E230x (Arm Cortex-M33) is built from the same GPIO, IOMUX, DMA, timer and UNICOMM
peripherals as the MSPM0 parts, and is supported by this HAL with the `am13e230*` chip features
(for example `am13e23019gpm` for the LP-AM13E230 LaunchPad). Build for `thumbv8m.main-none-eabihf`.

Tested on the LP-AM13E230: GPIO (including edge interrupts), the UNICOMM instances as UARTs
(blocking and buffered/async), DMA, the time driver (TIMG4 on LFCLK), the window watchdog and the
reset cause. CLK_OUT builds but is untested. The ADC, I2C and SPI of the UNICOMM instances, timer
PWM, low power modes and clock configuration (the parts run from SYSOSC at 32 MHz, the reset
configuration) are not implemented yet.

The time driver counts LFCLK, which runs from the internal LFOSC (32.768 kHz, +-3%), so time is
only as accurate as LFOSC. Use the executor of `embassy-executor` (`platform-cortex-m`), not the
`executor-*` features of this crate, which need `low-power`.

See [examples/am13e23019](https://github.com/nilabhanand-ti/embassy/tree/am13e2/examples/am13e23019).

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
