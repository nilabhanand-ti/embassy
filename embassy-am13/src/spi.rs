//! Serial Peripheral Interface (SPI) controller, on the UNICOMM instances which have one (UC0,
//! UC1, UC3 and UC4).
//!
//! The driver runs the bus in Motorola 3-wire format: drive chip selects with GPIO, for example with
//! the `SpiDevice` implementations of `embassy-embedded-hal`.

#![macro_use]

use core::future::poll_fn;
use core::marker::PhantomData;
use core::sync::atomic::{AtomicU8, Ordering};
use core::task::Poll;

use embassy_embedded_hal::SetConfig;
use embassy_hal_internal::PeripheralType;
use embassy_sync::waitqueue::AtomicWaker;
pub use embedded_hal::spi::{MODE_0, MODE_1, MODE_2, MODE_3, Mode, Phase, Polarity};

use crate::gpio::{AnyPin, PfType, Pull, SealedPin};
use crate::interrupt::Interrupt;
use crate::interrupt::typelevel::{Binding, Interrupt as _};
use crate::mode::{Async, Blocking, Mode as IoMode};
use crate::pac::unicommspi::{Unicommspi as Regs, vals};
use crate::{Peri, sysctl, unicomm};

macro_rules! pending_pin {
    ($pin: ident, $ty: expr) => {{
        let pf = $pin.pf_num();
        PendingPin {
            pin: $pin.into(),
            pf,
            ty: $ty,
        }
    }};
}

/// Bit order of the words on the bus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum BitOrder {
    /// Most significant bit first, the usual order.
    MsbFirst,
    /// Least significant bit first.
    LsbFirst,
}

/// SPI configuration.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Config {
    /// The SCLK frequency, in Hz.
    ///
    /// The driver picks the fastest clock which does not exceed it. The highest is half the
    /// functional clock: 16 MHz while the parts run from SYSOSC.
    pub frequency: u32,

    /// Clock polarity and phase.
    pub mode: Mode,

    /// Bit order.
    pub bit_order: BitOrder,

    /// Pull resistor on the POCI input.
    pub poci_pull: Pull,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            frequency: 1_000_000,
            mode: MODE_0,
            bit_order: BitOrder::MsbFirst,
            poci_pull: Pull::None,
        }
    }
}

/// Error in the SPI configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[non_exhaustive]
pub enum ConfigError {
    /// The frequency is 0, or too low to reach with the clock dividers.
    InvalidFrequency,
}

impl core::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidFrequency => write!(f, "SPI frequency cannot be reached"),
        }
    }
}

impl core::error::Error for ConfigError {}

/// SPI error.
///
/// The controller cannot fail a transfer; the type is there for the `embedded-hal` traits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[non_exhaustive]
pub enum Error {}

impl core::fmt::Display for Error {
    fn fmt(&self, _f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match *self {}
    }
}

impl core::error::Error for Error {}

impl embedded_hal::spi::Error for Error {
    fn kind(&self) -> embedded_hal::spi::ErrorKind {
        match *self {}
    }
}

/// A word the SPI can transfer: `u8` or `u16`.
#[allow(private_bounds)]
pub trait Word: SealedWord + Copy + Default + 'static {}

trait SealedWord {
    const BITS: u8;
    fn to_u16(self) -> u16;
    fn from_u16(v: u16) -> Self;
}

impl SealedWord for u8 {
    const BITS: u8 = 8;
    fn to_u16(self) -> u16 {
        self as u16
    }
    fn from_u16(v: u16) -> Self {
        v as u8
    }
}
impl Word for u8 {}

impl SealedWord for u16 {
    const BITS: u8 = 16;
    fn to_u16(self) -> u16 {
        self
    }
    fn from_u16(v: u16) -> Self {
        v
    }
}
impl Word for u16 {}

/// SPI controller driver.
pub struct Spi<'d, M: IoMode> {
    info: &'static Info,
    state: &'static State,
    sclk: Option<Peri<'d, AnyPin>>,
    pico: Option<Peri<'d, AnyPin>>,
    poci: Option<Peri<'d, AnyPin>>,
    _phantom: PhantomData<M>,
}

impl<'d> Spi<'d, Blocking> {
    /// Create a blocking SPI driver.
    pub fn new_blocking<T: Instance>(
        peri: Peri<'d, T>,
        sclk: Peri<'d, impl SclkPin<T>>,
        pico: Peri<'d, impl PicoPin<T>>,
        poci: Peri<'d, impl PociPin<T>>,
        config: Config,
    ) -> Result<Self, ConfigError> {
        Self::new_inner(
            peri,
            Some(pending_pin!(sclk, PfType::output(Pull::None, false))),
            Some(pending_pin!(pico, PfType::output(Pull::None, false))),
            Some(pending_pin!(poci, PfType::input(config.poci_pull, false))),
            config,
        )
    }

    /// Create a blocking SPI driver which only transmits.
    pub fn new_blocking_txonly<T: Instance>(
        peri: Peri<'d, T>,
        sclk: Peri<'d, impl SclkPin<T>>,
        pico: Peri<'d, impl PicoPin<T>>,
        config: Config,
    ) -> Result<Self, ConfigError> {
        Self::new_inner(
            peri,
            Some(pending_pin!(sclk, PfType::output(Pull::None, false))),
            Some(pending_pin!(pico, PfType::output(Pull::None, false))),
            None,
            config,
        )
    }

    /// Create a blocking SPI driver which only receives.
    pub fn new_blocking_rxonly<T: Instance>(
        peri: Peri<'d, T>,
        sclk: Peri<'d, impl SclkPin<T>>,
        poci: Peri<'d, impl PociPin<T>>,
        config: Config,
    ) -> Result<Self, ConfigError> {
        Self::new_inner(
            peri,
            Some(pending_pin!(sclk, PfType::output(Pull::None, false))),
            None,
            Some(pending_pin!(poci, PfType::input(config.poci_pull, false))),
            config,
        )
    }
}

impl<'d> Spi<'d, Async> {
    /// Create an interrupt driven SPI driver.
    pub fn new<T: Instance>(
        peri: Peri<'d, T>,
        sclk: Peri<'d, impl SclkPin<T>>,
        pico: Peri<'d, impl PicoPin<T>>,
        poci: Peri<'d, impl PociPin<T>>,
        _irq: impl Binding<T::Interrupt, InterruptHandler<T>> + 'd,
        config: Config,
    ) -> Result<Self, ConfigError> {
        let this = Self::new_inner(
            peri,
            Some(pending_pin!(sclk, PfType::output(Pull::None, false))),
            Some(pending_pin!(pico, PfType::output(Pull::None, false))),
            Some(pending_pin!(poci, PfType::input(config.poci_pull, false))),
            config,
        )?;

        T::Interrupt::unpend();
        unsafe { T::Interrupt::enable() };

        Ok(this)
    }

    /// Create an interrupt driven SPI driver which only transmits.
    pub fn new_txonly<T: Instance>(
        peri: Peri<'d, T>,
        sclk: Peri<'d, impl SclkPin<T>>,
        pico: Peri<'d, impl PicoPin<T>>,
        _irq: impl Binding<T::Interrupt, InterruptHandler<T>> + 'd,
        config: Config,
    ) -> Result<Self, ConfigError> {
        let this = Self::new_inner(
            peri,
            Some(pending_pin!(sclk, PfType::output(Pull::None, false))),
            Some(pending_pin!(pico, PfType::output(Pull::None, false))),
            None,
            config,
        )?;

        T::Interrupt::unpend();
        unsafe { T::Interrupt::enable() };

        Ok(this)
    }

    /// Read words, transmitting zeros.
    pub async fn read<W: Word>(&mut self, words: &mut [W]) -> Result<(), Error> {
        self.prepare::<W>();
        for word in words.iter_mut() {
            *word = self.async_exchange(W::default()).await;
        }
        Ok(())
    }

    /// Write words, discarding what is received.
    pub async fn write<W: Word>(&mut self, words: &[W]) -> Result<(), Error> {
        self.prepare::<W>();
        for &word in words {
            self.async_exchange(word).await;
        }
        Ok(())
    }

    /// Write `write` while reading into `read`. The longer of the two sets the length of the
    /// transfer: zeros are sent past the end of `write`, and what is received past the end of
    /// `read` is discarded.
    pub async fn transfer<W: Word>(&mut self, read: &mut [W], write: &[W]) -> Result<(), Error> {
        self.prepare::<W>();
        for i in 0..read.len().max(write.len()) {
            let rx = self.async_exchange(write.get(i).copied().unwrap_or_default()).await;
            if let Some(word) = read.get_mut(i) {
                *word = rx;
            }
        }
        Ok(())
    }

    /// Write the words while replacing them with what is received.
    pub async fn transfer_in_place<W: Word>(&mut self, words: &mut [W]) -> Result<(), Error> {
        self.prepare::<W>();
        for word in words.iter_mut() {
            *word = self.async_exchange(*word).await;
        }
        Ok(())
    }

    async fn async_exchange<W: Word>(&mut self, word: W) -> W {
        let r = self.info.regs;

        r.txdata().write(|w| w.set_data(word.to_u16()));

        poll_fn(|cx| {
            self.state.waker.register(cx.waker());

            if !r.stat().read().rxfe() {
                return Poll::Ready(());
            }

            // The interrupt handler masks the event again and wakes the task.
            r.cpu_int().imask().modify(|w| w.set_rx(true));
            if !r.stat().read().rxfe() {
                r.cpu_int().imask().modify(|w| w.set_rx(false));
                return Poll::Ready(());
            }

            Poll::Pending
        })
        .await;

        W::from_u16(r.rxdata().read().data())
    }
}

impl<'d, M: IoMode> Spi<'d, M> {
    fn new_inner<T: Instance>(
        _peri: Peri<'d, T>,
        sclk: Option<PendingPin<'d>>,
        pico: Option<PendingPin<'d>>,
        poci: Option<PendingPin<'d>>,
        config: Config,
    ) -> Result<Self, ConfigError> {
        let info = T::info();
        let state = T::state();

        unicomm::enable(info.unicomm, unicomm::Mode::Spi);
        configure(info, state, &config)?;

        // The pins are connected once the controller drives their idle levels.
        let connect = |pin: Option<PendingPin<'d>>| pin.map(PendingPin::connect);

        Ok(Self {
            info,
            state,
            sclk: connect(sclk),
            pico: connect(pico),
            poci: connect(poci),
            _phantom: PhantomData,
        })
    }

    /// Change the configuration.
    pub fn set_config(&mut self, config: &Config) -> Result<(), ConfigError> {
        // POCI keeps the pull it was created with.
        configure(self.info, self.state, config)
    }

    /// Connect the controller's output to its input inside the UNICOMM instance (loopback mode),
    /// or disconnect it again. The pins keep being driven.
    ///
    /// Useful to test the driver without a peripheral.
    pub fn set_loopback(&mut self, enabled: bool) {
        self.info.regs.ctl1().modify(|w| w.set_lbm(enabled));
    }

    /// Read words, transmitting zeros.
    pub fn blocking_read<W: Word>(&mut self, words: &mut [W]) -> Result<(), Error> {
        self.prepare::<W>();
        for word in words.iter_mut() {
            *word = self.blocking_exchange(W::default());
        }
        Ok(())
    }

    /// Write words, discarding what is received.
    pub fn blocking_write<W: Word>(&mut self, words: &[W]) -> Result<(), Error> {
        self.prepare::<W>();
        for &word in words {
            self.blocking_exchange(word);
        }
        Ok(())
    }

    /// Write `write` while reading into `read`. The longer of the two sets the length of the
    /// transfer: zeros are sent past the end of `write`, and what is received past the end of
    /// `read` is discarded.
    pub fn blocking_transfer<W: Word>(&mut self, read: &mut [W], write: &[W]) -> Result<(), Error> {
        self.prepare::<W>();
        for i in 0..read.len().max(write.len()) {
            let rx = self.blocking_exchange(write.get(i).copied().unwrap_or_default());
            if let Some(word) = read.get_mut(i) {
                *word = rx;
            }
        }
        Ok(())
    }

    /// Write the words while replacing them with what is received.
    pub fn blocking_transfer_in_place<W: Word>(&mut self, words: &mut [W]) -> Result<(), Error> {
        self.prepare::<W>();
        for word in words.iter_mut() {
            *word = self.blocking_exchange(*word);
        }
        Ok(())
    }

    /// Wait until the controller is idle.
    ///
    /// Every transfer waits for its last word to be received, so there is nothing left to wait for
    /// when it returns.
    pub fn blocking_flush(&mut self) -> Result<(), Error> {
        while self.info.regs.stat().read().busy() {}
        Ok(())
    }

    /// Set the word size and empty the RX FIFO before a transfer.
    fn prepare<W: Word>(&mut self) {
        let r = self.info.regs;

        if self.state.word_bits.load(Ordering::Relaxed) != W::BITS {
            // The frame format may only change while the controller is disabled.
            r.ctl1().modify(|w| w.set_enable(false));
            r.ctl0().modify(|w| w.set_dss(vals::Dss::from_bits(W::BITS - 1)));
            r.ctl1().modify(|w| w.set_enable(true));
            self.state.word_bits.store(W::BITS, Ordering::Relaxed);
        }

        while !r.stat().read().rxfe() {
            let _ = r.rxdata().read();
        }
    }

    /// Send one word and return the word received with it.
    ///
    /// Only one word is in flight at a time, so the RX FIFO cannot overflow.
    fn blocking_exchange<W: Word>(&mut self, word: W) -> W {
        let r = self.info.regs;

        r.txdata().write(|w| w.set_data(word.to_u16()));
        while r.stat().read().rxfe() {}
        W::from_u16(r.rxdata().read().data())
    }
}

impl<'d, M: IoMode> Drop for Spi<'d, M> {
    fn drop(&mut self) {
        self.info.regs.cpu_int().imask().write(|_| {});
        self.info.regs.ctl1().modify(|w| w.set_enable(false));

        self.sclk.as_ref().map(|x| x.set_as_disconnected());
        self.pico.as_ref().map(|x| x.set_as_disconnected());
        self.poci.as_ref().map(|x| x.set_as_disconnected());
    }
}

/// The dividers for a frequency: CLKDIV.RATIO and CLKCTL.SCR.
///
/// SCLK = functional clock / (RATIO + 1) / (2 * (SCR + 1)) (dl_unicommspi.h, DL_SPI_setBitRateSerialClockDivider).
fn dividers(clock: u32, frequency: u32) -> Result<(u8, u16), ConfigError> {
    if frequency == 0 {
        return Err(ConfigError::InvalidFrequency);
    }

    for ratio in 1..=8u32 {
        let clk = clock / ratio;
        // The smallest SCR whose frequency does not exceed the requested one.
        let scr = clk.div_ceil(2 * frequency).max(1) - 1;
        if scr <= 1023 {
            return Ok(((ratio - 1) as u8, scr as u16));
        }
    }

    Err(ConfigError::InvalidFrequency)
}

fn configure(info: &Info, state: &State, config: &Config) -> Result<(), ConfigError> {
    let r = info.regs;

    let (ratio, scr) = dividers(sysctl::mclk_div2_frequency(), config.frequency)?;

    r.ctl1().modify(|w| w.set_enable(false));

    // The functional clock is MCLK/2, the only clock of a UNICOMM instance.
    r.clksel().write(|w| w.set_busclk_sel(true));
    r.clkdiv().write(|w| w.set_ratio(ratio));
    r.clkctl().write(|w| {
        w.set_scr(scr);
        w.set_dsample(0);
    });

    r.ctl0().write(|w| {
        w.set_dss(vals::Dss::Dss8);
        w.set_frf(vals::Frf::Motorola3wire);
        w.set_spo(config.mode.polarity == Polarity::IdleHigh);
        w.set_sph(config.mode.phase == Phase::CaptureOnSecondTransition);
    });
    state.word_bits.store(8, Ordering::Relaxed);

    r.ifls().write(|w| {
        w.set_txiflsel(vals::Txiflsel::LvlNotFull);
        w.set_rxiflsel(vals::Rxiflsel::LvlNotEmpty);
        w.set_txclr(true);
        w.set_rxclr(true);
    });

    r.cpu_int().imask().write(|_| {});

    // Keep the loopback setting across reconfiguration.
    let lbm = r.ctl1().read().lbm();
    r.ctl1().write(|w| {
        w.set_cp(true);
        w.set_msb(config.bit_order == BitOrder::MsbFirst);
        w.set_lbm(lbm);
        w.set_enable(true);
    });

    Ok(())
}

/// A pin given to the driver, connected to the instance once it is configured.
struct PendingPin<'d> {
    pin: Peri<'d, AnyPin>,
    pf: u8,
    ty: PfType,
}

impl<'d> PendingPin<'d> {
    fn connect(self) -> Peri<'d, AnyPin> {
        self.pin.set_as_pf(self.pf, self.ty);
        self.pin
    }
}

/// SPI interrupt handler.
pub struct InterruptHandler<T: Instance> {
    _phantom: PhantomData<T>,
}

impl<T: Instance> crate::interrupt::typelevel::Handler<T::Interrupt> for InterruptHandler<T> {
    unsafe fn on_interrupt() {
        let r = T::info().regs;

        if r.cpu_int().mis().read().rx() {
            r.cpu_int().imask().modify(|w| w.set_rx(false));
            T::state().waker.wake();
        }
    }
}

impl<'d, M: IoMode> SetConfig for Spi<'d, M> {
    type Config = Config;
    type ConfigError = ConfigError;

    fn set_config(&mut self, config: &Self::Config) -> Result<(), Self::ConfigError> {
        self.set_config(config)
    }
}

impl<'d, M: IoMode> embedded_hal::spi::ErrorType for Spi<'d, M> {
    type Error = Error;
}

impl<'d, M: IoMode, W: Word> embedded_hal::spi::SpiBus<W> for Spi<'d, M> {
    fn read(&mut self, words: &mut [W]) -> Result<(), Self::Error> {
        self.blocking_read(words)
    }

    fn write(&mut self, words: &[W]) -> Result<(), Self::Error> {
        self.blocking_write(words)
    }

    fn transfer(&mut self, read: &mut [W], write: &[W]) -> Result<(), Self::Error> {
        self.blocking_transfer(read, write)
    }

    fn transfer_in_place(&mut self, words: &mut [W]) -> Result<(), Self::Error> {
        self.blocking_transfer_in_place(words)
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        self.blocking_flush()
    }
}

impl<'d, W: Word> embedded_hal_async::spi::SpiBus<W> for Spi<'d, Async> {
    async fn read(&mut self, words: &mut [W]) -> Result<(), Self::Error> {
        Spi::read(self, words).await
    }

    async fn write(&mut self, words: &[W]) -> Result<(), Self::Error> {
        Spi::write(self, words).await
    }

    async fn transfer(&mut self, read: &mut [W], write: &[W]) -> Result<(), Self::Error> {
        Spi::transfer(self, read, write).await
    }

    async fn transfer_in_place(&mut self, words: &mut [W]) -> Result<(), Self::Error> {
        Spi::transfer_in_place(self, words).await
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        self.blocking_flush()
    }
}

pub(crate) struct Info {
    pub(crate) regs: Regs,
    pub(crate) unicomm: crate::pac::unicomm::Unicomm,
    #[allow(dead_code)]
    pub(crate) interrupt: Interrupt,
}

pub(crate) struct State {
    waker: AtomicWaker,
    /// The word size CTL0.DSS is set to.
    word_bits: AtomicU8,
}

impl State {
    pub(crate) const fn new() -> Self {
        Self {
            waker: AtomicWaker::new(),
            word_bits: AtomicU8::new(0),
        }
    }
}

pub(crate) trait SealedInstance {
    fn info() -> &'static Info;
    fn state() -> &'static State;
}

/// A UNICOMM instance which can be an SPI controller.
#[allow(private_bounds)]
pub trait Instance: SealedInstance + PeripheralType {
    /// The interrupt of the instance.
    type Interrupt: crate::interrupt::typelevel::Interrupt;
}

/// SCLK pin.
pub trait SclkPin<T: Instance>: crate::gpio::Pin {
    /// The PF number of the pin's SCLK function.
    fn pf_num(&self) -> u8;
}

/// PICO (controller out, peripheral in) pin.
pub trait PicoPin<T: Instance>: crate::gpio::Pin {
    /// The PF number of the pin's PICO function.
    fn pf_num(&self) -> u8;
}

/// POCI (peripheral out, controller in) pin.
pub trait PociPin<T: Instance>: crate::gpio::Pin {
    /// The PF number of the pin's POCI function.
    fn pf_num(&self) -> u8;
}

macro_rules! impl_spi_instance {
    // `$instance` is the UNICOMM instance, `$spi` its SPI registers.
    ($instance: ident, $spi: ident) => {
        impl crate::spi::SealedInstance for crate::peripherals::$instance {
            fn info() -> &'static crate::spi::Info {
                use crate::interrupt::typelevel::Interrupt;

                const INFO: crate::spi::Info = crate::spi::Info {
                    regs: crate::pac::$spi,
                    unicomm: crate::pac::$instance,
                    interrupt: crate::interrupt::typelevel::$instance::IRQ,
                };
                &INFO
            }

            fn state() -> &'static crate::spi::State {
                static STATE: crate::spi::State = crate::spi::State::new();
                &STATE
            }
        }

        impl crate::spi::Instance for crate::peripherals::$instance {
            type Interrupt = crate::interrupt::typelevel::$instance;
        }
    };
}

macro_rules! impl_spi_sclk_pin {
    ($instance: ident, $pin: ident, $pf: expr) => {
        impl crate::spi::SclkPin<crate::peripherals::$instance> for crate::peripherals::$pin {
            fn pf_num(&self) -> u8 {
                $pf
            }
        }
    };
}

macro_rules! impl_spi_pico_pin {
    ($instance: ident, $pin: ident, $pf: expr) => {
        impl crate::spi::PicoPin<crate::peripherals::$instance> for crate::peripherals::$pin {
            fn pf_num(&self) -> u8 {
                $pf
            }
        }
    };
}

macro_rules! impl_spi_poci_pin {
    ($instance: ident, $pin: ident, $pf: expr) => {
        impl crate::spi::PociPin<crate::peripherals::$instance> for crate::peripherals::$pin {
            fn pf_num(&self) -> u8 {
                $pf
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dividers_32mhz() {
        // 16 MHz is the fastest: SCR 0.
        assert_eq!(dividers(32_000_000, 16_000_000), Ok((0, 0)));
        // Never faster than asked.
        assert_eq!(dividers(32_000_000, 1_000_000), Ok((0, 15)));
        assert_eq!(dividers(32_000_000, 3_000_000), Ok((0, 5)));
        // Below 32 MHz / 2048 the prescaler is needed.
        assert_eq!(dividers(32_000_000, 10_000), Ok((1, 799)));
        assert_eq!(dividers(32_000_000, 0), Err(ConfigError::InvalidFrequency));
        assert_eq!(dividers(32_000_000, 1), Err(ConfigError::InvalidFrequency));
    }
}
