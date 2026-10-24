//! System controller (SYSCTL) driver.

#![macro_use]

use crate::gpio::{AnyPin, PfType, Pin, Pull, SealedPin};
use crate::pac::sysctl::vals;
use crate::peripherals::CLK_OUT;
use crate::{Peri, pac};

/// Frequency of MCLK. The AM13E2x runs MCLK from SYSOSC at its 32 MHz base frequency out of reset.
///
/// While MCLK comes from SYSOSC, SYSCTL bypasses the MCLK/2 and MCLK/4 dividers, so the MCLK/2
/// clock of the UNICOMM instances and the MCLK/4 bus clock (ULPCLK) run at MCLK too (AM13E230x TRM,
/// section 3.4.3.3).
// TODO: Compute these once SYSPLL/HFCLK can be configured.
pub(crate) fn mclk_frequency() -> u32 {
    32_000_000
}

/// Frequency of MCLK/2, the functional clock of the UNICOMM instances.
pub(crate) fn mclk_div2_frequency() -> u32 {
    mclk_frequency()
}

/// Source and configuration for CLK_OUT pin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ClkOutSource {
    /// Use SYSOSC as the source.
    ///
    /// The divider is optional for this clock source.
    Sysosc(Option<ClkOutDiv>),

    /// Use ULPCLK (the MCLK/4 bus clock) as the source.
    ///
    /// The divider is required for this clock source.
    UlpClk(ClkOutDiv),

    /// Use LFCLK as the source.
    ///
    /// The divider is optional for this clock source.
    LfClk(Option<ClkOutDiv>),

    /// Use HFCLK as the source.
    ///
    /// The divider is optional for this clock source.
    Hfclk(Option<ClkOutDiv>),

    /// Use SYSPLLCLK1 as the source.
    ///
    /// The divider is optional for this clock source.
    SysPllClk1(Option<ClkOutDiv>),
}

impl ClkOutSource {
    fn convert_div(self) -> (bool, vals::Exclkdivval) {
        match self {
            ClkOutSource::Sysosc(div) => div_to_pac(div),
            ClkOutSource::UlpClk(div) => div_to_pac(Some(div)),
            ClkOutSource::LfClk(div) => div_to_pac(div),
            ClkOutSource::Hfclk(div) => div_to_pac(div),
            ClkOutSource::SysPllClk1(div) => div_to_pac(div),
        }
    }

    fn convert_src(self) -> vals::Exclksrc {
        match self {
            ClkOutSource::Sysosc(_) => vals::Exclksrc::Sysosc,
            ClkOutSource::UlpClk(_) => vals::Exclksrc::Ulpclk,
            ClkOutSource::LfClk(_) => vals::Exclksrc::Lfclk,
            ClkOutSource::Hfclk(_) => vals::Exclksrc::Hfclk,
            ClkOutSource::SysPllClk1(_) => vals::Exclksrc::Syspllout1,
        }
    }
}

/// Divider applied to the clock source of the CLK_OUT pin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ClkOutDiv {
    /// Divide by 2.
    Div2,

    /// Divide by 4.
    Div4,

    /// Divide by 6.
    Div6,

    /// Divide by 8.
    Div8,

    /// Divide by 10.
    Div10,

    /// Divide by 12.
    Div12,

    /// Divide by 14.
    Div14,

    /// Divide by 16.
    Div16,
}

/// CLK_OUT pin driver.
pub struct ClkOut<'d> {
    pin: Peri<'d, AnyPin>,
}

impl<'d> ClkOut<'d> {
    /// Create a bew CLK_OUT instance.
    pub fn new(_peri: Peri<'d, CLK_OUT>, pin: Peri<'d, impl ClkOutPin>, source: ClkOutSource) -> Self {
        // FIXME: Config (pull, invert, etc?)
        let pf = PfType::output(Pull::None, false);
        // FIXME: Infallible operation
        let pin = unwrap!(new_pin!(pin, pf));

        let (en_div, div) = source.convert_div();
        let src = source.convert_src();
        pac::SYSCTL.genclkcfg().modify(|w| {
            w.set_exclksrc(src);
            w.set_exclkdivval(div);
            w.set_exclkdiven(en_div);
        });

        pac::SYSCTL.genclken().modify(|w| {
            w.set_exclken(true);
        });

        Self { pin }
    }
}

impl<'d> Drop for ClkOut<'d> {
    fn drop(&mut self) {
        pac::SYSCTL.genclken().modify(|w| {
            w.set_exclken(false);
        });

        self.pin.set_as_disconnected();
    }
}

/// ClkOut pin trait.
pub trait ClkOutPin: Pin {
    /// Get the PF number needed to use this pin aas ClkOut pin.
    fn pf_num(&self) -> u8;
}

macro_rules! impl_clk_out_pin {
    ($pin: ident, $pf: expr) => {
        impl crate::sysctl::ClkOutPin for $crate::peripherals::$pin {
            fn pf_num(&self) -> u8 {
                $pf
            }
        }
    };
}

/// (DIVEN, DIVVAL)
fn div_to_pac(div: Option<ClkOutDiv>) -> (bool, vals::Exclkdivval) {
    match div {
        Some(ClkOutDiv::Div2) => (true, vals::Exclkdivval::Div2),
        Some(ClkOutDiv::Div4) => (true, vals::Exclkdivval::Div4),
        Some(ClkOutDiv::Div6) => (true, vals::Exclkdivval::Div6),
        Some(ClkOutDiv::Div8) => (true, vals::Exclkdivval::Div8),
        Some(ClkOutDiv::Div10) => (true, vals::Exclkdivval::Div10),
        Some(ClkOutDiv::Div12) => (true, vals::Exclkdivval::Div12),
        Some(ClkOutDiv::Div14) => (true, vals::Exclkdivval::Div14),
        Some(ClkOutDiv::Div16) => (true, vals::Exclkdivval::Div16),
        // divider is ignored. set to default value
        None => (false, vals::Exclkdivval::Div2),
    }
}
