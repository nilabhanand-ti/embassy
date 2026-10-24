//! The UNICOMM wrapper: power, reset and the selection of the mode an instance runs in.

use crate::pac::unicomm::{Unicomm, vals};

/// The mode of a UNICOMM instance.
#[derive(Clone, Copy)]
pub(crate) enum Mode {
    Uart,
    Spi,
    I2cController,
}

/// Reset and power up a UNICOMM instance, and select its mode.
pub(crate) fn enable(unicomm: Unicomm, mode: Mode) {
    let gprcm = unicomm.gprcm(0);

    gprcm.rstctl().write(|w| {
        w.set_resetstkyclr(true);
        w.set_resetassert(true);
        w.set_key(vals::ResetKey::Key);
    });

    gprcm.pwren().write(|w| {
        w.set_enable(true);
        w.set_key(vals::PwrenKey::Key);
    });

    // The registers of the instance are not accessible for a few cycles after power up. The SDK
    // waits 16 cycles.
    cortex_m::asm::delay(16);

    unicomm.ipmode().write(|w| {
        w.set_select(match mode {
            Mode::Uart => vals::Select::Uart,
            Mode::Spi => vals::Select::Spi,
            Mode::I2cController => vals::Select::I2cController,
        })
    });
}
