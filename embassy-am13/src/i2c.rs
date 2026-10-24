//! Inter-Integrated Circuit (I2C) controller, on the UNICOMM instances with burst support (UC2
//! and UC5).
//!
//! UC2 and UC5 are the "advanced" UNICOMM instances (dl_unicomm.h,
//! `DL_UNICOMM_isAdvanceFeaturesEnabled`): their I2C controller runs a whole transfer of up to 4095
//! bytes (CTR.BLEN) on its own, while the driver streams the bytes through the FIFO. The other
//! instances move one byte per transfer, which this driver does not support.

#![macro_use]

use core::future::poll_fn;
use core::marker::PhantomData;
use core::task::Poll;

use embassy_embedded_hal::SetConfig;
use embassy_hal_internal::PeripheralType;
use embassy_sync::waitqueue::AtomicWaker;

use crate::gpio::{AnyPin, PfType, Pull, SealedPin};
use crate::interrupt::Interrupt;
use crate::interrupt::typelevel::{Binding, Interrupt as _};
use crate::mode::{Async, Blocking, Mode};
use crate::pac::unicommi2cc::{Unicommi2cc as Regs, vals};
use crate::{Peri, pac, sysctl, unicomm};

/// The longest transfer: CTR.BLEN is 12 bits.
const MAX_LEN: usize = 4095;

/// I2C configuration.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Config {
    /// The SCL frequency, in Hz: up to 100 kHz (standard mode), 400 kHz (fast mode) or 1 MHz
    /// (fast mode plus).
    ///
    /// The driver picks the fastest SCL which does not exceed it.
    pub frequency: u32,

    /// Pull resistor on SDA. The bus needs pull-ups; the internal ones are weak, use external
    /// resistors where possible.
    pub sda_pull: Pull,

    /// Pull resistor on SCL.
    pub scl_pull: Pull,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            frequency: 100_000,
            sda_pull: Pull::None,
            scl_pull: Pull::None,
        }
    }
}

impl Config {
    fn sda_pf(&self) -> PfType {
        PfType::input(self.sda_pull, false)
    }

    fn scl_pf(&self) -> PfType {
        PfType::input(self.scl_pull, false)
    }
}

/// Error in the I2C configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[non_exhaustive]
pub enum ConfigError {
    /// The frequency is 0, above 1 MHz, or too low to reach with the clock dividers.
    InvalidFrequency,
}

impl core::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidFrequency => write!(f, "I2C frequency cannot be reached"),
        }
    }
}

impl core::error::Error for ConfigError {}

/// I2C error.
#[derive(Debug, Eq, PartialEq, Copy, Clone)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[non_exhaustive]
pub enum Error {
    /// The target did not acknowledge its address.
    AddressNack,

    /// The target did not acknowledge a data byte.
    DataNack,

    /// Arbitration lost to another controller.
    Arbitration,

    /// Zero-length transfers are not supported.
    ZeroLengthTransfer,

    /// A transfer is limited to 4095 bytes.
    TransferTooLong,
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let message = match self {
            Self::AddressNack => "Address not acknowledged",
            Self::DataNack => "Data not acknowledged",
            Self::Arbitration => "Arbitration lost",
            Self::ZeroLengthTransfer => "Zero-length transfers are not supported",
            Self::TransferTooLong => "Transfer longer than 4095 bytes",
        };

        write!(f, "{}", message)
    }
}

impl core::error::Error for Error {}

impl embedded_hal::i2c::Error for Error {
    fn kind(&self) -> embedded_hal::i2c::ErrorKind {
        use embedded_hal::i2c::{ErrorKind, NoAcknowledgeSource};

        match self {
            Self::AddressNack => ErrorKind::NoAcknowledge(NoAcknowledgeSource::Address),
            Self::DataNack => ErrorKind::NoAcknowledge(NoAcknowledgeSource::Data),
            Self::Arbitration => ErrorKind::ArbitrationLoss,
            Self::ZeroLengthTransfer | Self::TransferTooLong => ErrorKind::Other,
        }
    }
}

/// Direction of a transfer.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Dir {
    Write,
    Read,
}

/// I2C controller driver.
pub struct I2c<'d, M: Mode> {
    info: &'static Info,
    state: &'static State,
    scl: Peri<'d, AnyPin>,
    sda: Peri<'d, AnyPin>,
    _phantom: PhantomData<M>,
}

impl<'d> I2c<'d, Blocking> {
    /// Create a blocking I2C controller driver.
    pub fn new_blocking<T: Instance>(
        peri: Peri<'d, T>,
        scl: Peri<'d, impl SclPin<T>>,
        sda: Peri<'d, impl SdaPin<T>>,
        config: Config,
    ) -> Result<Self, ConfigError> {
        Self::new_inner(peri, scl, sda, config)
    }
}

impl<'d> I2c<'d, Async> {
    /// Create an interrupt driven I2C controller driver.
    pub fn new<T: Instance>(
        peri: Peri<'d, T>,
        scl: Peri<'d, impl SclPin<T>>,
        sda: Peri<'d, impl SdaPin<T>>,
        _irq: impl Binding<T::Interrupt, InterruptHandler<T>> + 'd,
        config: Config,
    ) -> Result<Self, ConfigError> {
        let this = Self::new_inner(peri, scl, sda, config)?;

        T::Interrupt::unpend();
        unsafe { T::Interrupt::enable() };

        Ok(this)
    }

    /// Read from the target at `address`.
    pub async fn read(&mut self, address: u8, read: &mut [u8]) -> Result<(), Error> {
        check_len(read.len())?;
        self.wait_bus_free();
        self.async_read_inner(address, read, true).await
    }

    /// Write to the target at `address`.
    pub async fn write(&mut self, address: u8, write: &[u8]) -> Result<(), Error> {
        check_len(write.len())?;
        self.wait_bus_free();
        self.async_write_inner(address, write, true).await
    }

    /// Write to the target at `address`, then read from it after a repeated START.
    pub async fn write_read(&mut self, address: u8, write: &[u8], read: &mut [u8]) -> Result<(), Error> {
        check_len(write.len())?;
        check_len(read.len())?;
        self.wait_bus_free();
        self.async_write_inner(address, write, false).await?;
        self.async_read_inner(address, read, true).await
    }

    async fn async_write_inner(&mut self, address: u8, write: &[u8], stop: bool) -> Result<(), Error> {
        let r = self.info.regs;
        let mut bytes = write.iter();

        self.flush_fifos();
        fill_tx(r, &mut bytes);
        self.start(address, Dir::Write, write.len(), stop);

        loop {
            fill_tx(r, &mut bytes);
            if !r.sr().read().busy() {
                break;
            }
            // Wake on space in the TX FIFO while bytes remain, and on the end of the transfer.
            self.wait_event(bytes.len() != 0, false).await;
        }

        self.finish()
    }

    async fn async_read_inner(&mut self, address: u8, read: &mut [u8], stop: bool) -> Result<(), Error> {
        let r = self.info.regs;
        let len = read.len();
        let mut received = 0;

        self.start(address, Dir::Read, len, stop);

        loop {
            drain_rx(r, read, &mut received);
            if !r.sr().read().busy() {
                drain_rx(r, read, &mut received);
                break;
            }
            self.wait_event(false, received < len).await;
        }

        self.finish()
    }

    /// Wait for one of the controller's events. The interrupt handler masks the events and wakes
    /// the task; the caller then looks at the status registers.
    async fn wait_event(&mut self, tx: bool, rx: bool) {
        let r = self.info.regs;

        poll_fn(|cx| {
            self.state.waker.register(cx.waker());

            if self.state.pending.swap(false, core::sync::atomic::Ordering::AcqRel) {
                return Poll::Ready(());
            }

            r.cpu_int().iclr().write(|w| {
                w.set_txtrg(true);
                w.set_rxtrg(true);
            });
            r.cpu_int().imask().write(|w| {
                w.set_txtrg(tx);
                w.set_rxtrg(rx);
                w.set_txdone(true);
                w.set_rxdone(true);
                w.set_nack(true);
                w.set_arblost(true);
                w.set_stop(true);
            });

            // The transfer may have finished, or the FIFO changed, before the events were unmasked.
            let sr = r.sr().read();
            if !sr.busy() || (tx && !sr.txff()) || (rx && !sr.rxfe()) {
                r.cpu_int().imask().write(|_| {});
                return Poll::Ready(());
            }

            Poll::Pending
        })
        .await
    }
}

impl<'d, M: Mode> I2c<'d, M> {
    fn new_inner<T: Instance>(
        _peri: Peri<'d, T>,
        scl: Peri<'d, impl SclPin<T>>,
        sda: Peri<'d, impl SdaPin<T>>,
        config: Config,
    ) -> Result<Self, ConfigError> {
        let info = T::info();
        let state = T::state();

        let tpr = timer_period(sysctl::mclk_div2_frequency(), config.frequency)?;

        unicomm::enable(info.unicomm, unicomm::Mode::I2cController);
        configure(info, tpr);

        // Connect the pins once the controller is enabled, as open-drain outputs.
        let scl_pf = scl.pf_num();
        let sda_pf = sda.pf_num();
        let scl: Peri<'d, AnyPin> = scl.into();
        let sda: Peri<'d, AnyPin> = sda.into();
        connect(&scl, scl_pf, config.scl_pf());
        connect(&sda, sda_pf, config.sda_pf());

        Ok(Self {
            info,
            state,
            scl,
            sda,
            _phantom: PhantomData,
        })
    }

    /// Change the configuration.
    pub fn set_config(&mut self, config: &Config) -> Result<(), ConfigError> {
        let tpr = timer_period(sysctl::mclk_div2_frequency(), config.frequency)?;

        // The pulls are changed on the pins as they are.
        pac::IOMUX.pincm(self.scl._pin_cm() as usize).modify(|w| {
            w.set_pipu(config.scl_pull == Pull::Up);
            w.set_pipd(config.scl_pull == Pull::Down);
        });
        pac::IOMUX.pincm(self.sda._pin_cm() as usize).modify(|w| {
            w.set_pipu(config.sda_pull == Pull::Up);
            w.set_pipd(config.sda_pull == Pull::Down);
        });

        configure(self.info, tpr);
        Ok(())
    }

    /// Read from the target at `address`.
    pub fn blocking_read(&mut self, address: u8, read: &mut [u8]) -> Result<(), Error> {
        check_len(read.len())?;
        self.wait_bus_free();
        self.blocking_read_inner(address, read, true)
    }

    /// Write to the target at `address`.
    pub fn blocking_write(&mut self, address: u8, write: &[u8]) -> Result<(), Error> {
        check_len(write.len())?;
        self.wait_bus_free();
        self.blocking_write_inner(address, write, true)
    }

    /// Write to the target at `address`, then read from it after a repeated START.
    pub fn blocking_write_read(&mut self, address: u8, write: &[u8], read: &mut [u8]) -> Result<(), Error> {
        check_len(write.len())?;
        check_len(read.len())?;
        self.wait_bus_free();
        self.blocking_write_inner(address, write, false)?;
        self.blocking_read_inner(address, read, true)
    }

    /// Whether SCL and SDA are high, as the controller sees them (SCL, SDA).
    ///
    /// Both are high on an idle bus with its pull-ups.
    pub fn bus_levels(&self) -> (bool, bool) {
        let bmon = self.info.regs.bmon().read();
        (bmon.scl(), bmon.sda())
    }

    fn blocking_write_inner(&mut self, address: u8, write: &[u8], stop: bool) -> Result<(), Error> {
        let r = self.info.regs;
        let mut bytes = write.iter();

        self.flush_fifos();
        fill_tx(r, &mut bytes);
        self.start(address, Dir::Write, write.len(), stop);

        while r.sr().read().busy() {
            fill_tx(r, &mut bytes);
        }

        self.finish()
    }

    fn blocking_read_inner(&mut self, address: u8, read: &mut [u8], stop: bool) -> Result<(), Error> {
        let r = self.info.regs;
        let mut received = 0;

        self.start(address, Dir::Read, read.len(), stop);

        while r.sr().read().busy() {
            drain_rx(r, read, &mut received);
        }
        drain_rx(r, read, &mut received);

        self.finish()
    }

    /// Wait until no other controller uses the bus.
    fn wait_bus_free(&self) {
        while self.info.regs.sr().read().busbsy() {}
    }

    /// Start a transfer of `len` bytes, with a START (or repeated START) and, if `stop`, a STOP.
    fn start(&mut self, address: u8, dir: Dir, len: usize, stop: bool) {
        let r = self.info.regs;

        r.ta().write(|w| {
            w.set_addr(address as u16);
            w.set_dir(dir == Dir::Read);
            // 7-bit addressing.
            w.set_mode(false);
        });

        // The burst is set up first and started with FRM_START, as the SDK does
        // (DL_I2CC_startTransferAdvanced).
        r.ctr().write(|w| {
            w.set_blen(len as u16);
            w.set_start(true);
            w.set_stop(stop);
            // NACK the last byte of a read.
            w.set_ack(false);
        });
        cortex_m::asm::dsb();
        r.ctr().modify(|w| w.set_frm_start(true));
        cortex_m::asm::dsb();

        // BUSY follows FRM_START after a few cycles; the callers wait for it to clear.
        for _ in 0..64 {
            if r.sr().read().busy() {
                break;
            }
        }
    }

    /// Check the outcome of a transfer once the controller is no longer busy, and clean up after
    /// an error.
    fn finish(&mut self) -> Result<(), Error> {
        let r = self.info.regs;
        let sr = r.sr().read();

        let result = if sr.arblst() {
            Err(Error::Arbitration)
        } else if sr.err() && sr.adrack() {
            Err(Error::AddressNack)
        } else if sr.err() {
            Err(Error::DataNack)
        } else {
            Ok(())
        };

        if result.is_err() {
            // Release the bus if the controller still holds it, and drop what is left in the
            // FIFOs so it does not go out with the next transfer.
            if r.sr().read().busbsy() && !sr.arblst() {
                r.ctr().write(|w| {
                    w.set_blen(0);
                    w.set_stop(true);
                });
                r.ctr().modify(|w| w.set_frm_start(true));
                while r.sr().read().busy() {}
            }
            self.flush_fifos();
        }

        result
    }

    fn flush_fifos(&mut self) {
        let r = self.info.regs;

        r.ifls().modify(|w| {
            w.set_txclr(true);
            w.set_rxclr(true);
        });
        while !(r.sr().read().txclr() && r.sr().read().rxclr()) {}
        r.ifls().modify(|w| {
            w.set_txclr(false);
            w.set_rxclr(false);
        });
    }
}

impl<'d, M: Mode> Drop for I2c<'d, M> {
    fn drop(&mut self) {
        self.info.regs.cpu_int().imask().write(|_| {});
        self.info.regs.cr().modify(|w| w.set_enable(false));

        self.scl.set_as_disconnected();
        self.sda.set_as_disconnected();
    }
}

fn check_len(len: usize) -> Result<(), Error> {
    match len {
        0 => Err(Error::ZeroLengthTransfer),
        n if n > MAX_LEN => Err(Error::TransferTooLong),
        _ => Ok(()),
    }
}

/// Put bytes into the TX FIFO while there is room.
fn fill_tx<'a>(r: Regs, bytes: &mut impl ExactSizeIterator<Item = &'a u8>) {
    while bytes.len() != 0 && !r.sr().read().txff() {
        let byte = *bytes.next().unwrap();
        r.txdata().write(|w| w.set_data(byte));
    }
}

/// Take bytes out of the RX FIFO.
fn drain_rx(r: Regs, read: &mut [u8], received: &mut usize) {
    while !r.sr().read().rxfe() {
        let byte = r.rxdata().read().data();
        if let Some(slot) = read.get_mut(*received) {
            *slot = byte;
            *received += 1;
        }
    }
}

/// The timer period for an SCL frequency.
///
/// SCL period = (1 + TPR) * 10 functional clock periods (dl_unicommi2cc.h,
/// `DL_I2CC_setTimerPeriod`), with TPR from 1 to 127.
fn timer_period(clock: u32, frequency: u32) -> Result<u8, ConfigError> {
    if frequency == 0 || frequency > 1_000_000 {
        return Err(ConfigError::InvalidFrequency);
    }

    // The smallest TPR whose frequency does not exceed the requested one.
    let tpr = clock.div_ceil(10 * frequency).saturating_sub(1).max(1);
    if tpr > 127 {
        return Err(ConfigError::InvalidFrequency);
    }

    Ok(tpr as u8)
}

fn configure(info: &Info, tpr: u8) {
    let r = info.regs;

    r.cr().modify(|w| w.set_enable(false));

    // The functional clock is MCLK/2, the only clock of a UNICOMM instance.
    r.clksel().write(|w| w.set_busclk_sel(true));
    r.clkdiv().write(|w| w.set_ratio(0));
    r.tpr().write(|w| w.set_tpr(tpr));

    r.ctr().write(|_| {});
    r.ifls().write(|w| {
        w.set_txiflsel(vals::Txiflsel::LvlNotFull);
        w.set_rxiflsel(vals::Rxiflsel::LvlNotEmpty);
    });
    r.cpu_int().imask().write(|_| {});

    r.cr().write(|w| {
        w.set_clkstretch(true);
        w.set_enable(true);
    });
}

/// Connect a pin to the controller as an open-drain output (HIZ1) with its input enabled.
fn connect(pin: &Peri<'_, AnyPin>, pf: u8, ty: PfType) {
    pin.set_as_pf(pf, ty);
    pac::IOMUX.pincm(pin._pin_cm() as usize).modify(|w| w.set_hiz1(true));
}

/// I2C interrupt handler.
pub struct InterruptHandler<T: Instance> {
    _phantom: PhantomData<T>,
}

impl<T: Instance> crate::interrupt::typelevel::Handler<T::Interrupt> for InterruptHandler<T> {
    unsafe fn on_interrupt() {
        let r = T::info().regs;

        if r.cpu_int().mis().read().0 != 0 {
            r.cpu_int().imask().write(|_| {});
            T::state().pending.store(true, core::sync::atomic::Ordering::Release);
            T::state().waker.wake();
        }
    }
}

impl<'d, M: Mode> SetConfig for I2c<'d, M> {
    type Config = Config;
    type ConfigError = ConfigError;

    fn set_config(&mut self, config: &Self::Config) -> Result<(), Self::ConfigError> {
        self.set_config(config)
    }
}

impl<'d, M: Mode> embedded_hal::i2c::ErrorType for I2c<'d, M> {
    type Error = Error;
}

impl<'d, M: Mode> embedded_hal::i2c::I2c for I2c<'d, M> {
    fn read(&mut self, address: u8, read: &mut [u8]) -> Result<(), Self::Error> {
        self.blocking_read(address, read)
    }

    fn write(&mut self, address: u8, write: &[u8]) -> Result<(), Self::Error> {
        self.blocking_write(address, write)
    }

    fn write_read(&mut self, address: u8, write: &[u8], read: &mut [u8]) -> Result<(), Self::Error> {
        self.blocking_write_read(address, write, read)
    }

    fn transaction(
        &mut self,
        address: u8,
        operations: &mut [embedded_hal::i2c::Operation<'_>],
    ) -> Result<(), Self::Error> {
        self.wait_bus_free();
        let last = operations.len().saturating_sub(1);
        for (i, op) in operations.iter_mut().enumerate() {
            let stop = i == last;
            match op {
                embedded_hal::i2c::Operation::Read(read) => {
                    check_len(read.len())?;
                    self.blocking_read_inner(address, read, stop)?;
                }
                embedded_hal::i2c::Operation::Write(write) => {
                    check_len(write.len())?;
                    self.blocking_write_inner(address, write, stop)?;
                }
            }
        }
        Ok(())
    }
}

impl<'d> embedded_hal_async::i2c::I2c for I2c<'d, Async> {
    async fn read(&mut self, address: u8, read: &mut [u8]) -> Result<(), Self::Error> {
        I2c::read(self, address, read).await
    }

    async fn write(&mut self, address: u8, write: &[u8]) -> Result<(), Self::Error> {
        I2c::write(self, address, write).await
    }

    async fn write_read(&mut self, address: u8, write: &[u8], read: &mut [u8]) -> Result<(), Self::Error> {
        I2c::write_read(self, address, write, read).await
    }

    async fn transaction(
        &mut self,
        address: u8,
        operations: &mut [embedded_hal::i2c::Operation<'_>],
    ) -> Result<(), Self::Error> {
        self.wait_bus_free();
        let last = operations.len().saturating_sub(1);
        for (i, op) in operations.iter_mut().enumerate() {
            let stop = i == last;
            match op {
                embedded_hal::i2c::Operation::Read(read) => {
                    check_len(read.len())?;
                    self.async_read_inner(address, read, stop).await?;
                }
                embedded_hal::i2c::Operation::Write(write) => {
                    check_len(write.len())?;
                    self.async_write_inner(address, write, stop).await?;
                }
            }
        }
        Ok(())
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
    /// An event fired since the task last looked.
    pending: core::sync::atomic::AtomicBool,
}

impl State {
    pub(crate) const fn new() -> Self {
        Self {
            waker: AtomicWaker::new(),
            pending: core::sync::atomic::AtomicBool::new(false),
        }
    }
}

pub(crate) trait SealedInstance {
    fn info() -> &'static Info;
    fn state() -> &'static State;
}

/// A UNICOMM instance which can be an I2C controller.
#[allow(private_bounds)]
pub trait Instance: SealedInstance + PeripheralType {
    /// The interrupt of the instance.
    type Interrupt: crate::interrupt::typelevel::Interrupt;
}

/// SCL pin.
pub trait SclPin<T: Instance>: crate::gpio::Pin {
    /// The PF number of the pin's SCL function.
    fn pf_num(&self) -> u8;
}

/// SDA pin.
pub trait SdaPin<T: Instance>: crate::gpio::Pin {
    /// The PF number of the pin's SDA function.
    fn pf_num(&self) -> u8;
}

macro_rules! impl_i2c_instance {
    // `$instance` is the UNICOMM instance, `$i2c` its I2C controller registers.
    ($instance: ident, $i2c: ident) => {
        impl crate::i2c::SealedInstance for crate::peripherals::$instance {
            fn info() -> &'static crate::i2c::Info {
                use crate::interrupt::typelevel::Interrupt;

                const INFO: crate::i2c::Info = crate::i2c::Info {
                    regs: crate::pac::$i2c,
                    unicomm: crate::pac::$instance,
                    interrupt: crate::interrupt::typelevel::$instance::IRQ,
                };
                &INFO
            }

            fn state() -> &'static crate::i2c::State {
                static STATE: crate::i2c::State = crate::i2c::State::new();
                &STATE
            }
        }

        impl crate::i2c::Instance for crate::peripherals::$instance {
            type Interrupt = crate::interrupt::typelevel::$instance;
        }
    };
}

macro_rules! impl_i2c_scl_pin {
    ($instance: ident, $pin: ident, $pf: expr) => {
        impl crate::i2c::SclPin<crate::peripherals::$instance> for crate::peripherals::$pin {
            fn pf_num(&self) -> u8 {
                $pf
            }
        }
    };
}

macro_rules! impl_i2c_sda_pin {
    ($instance: ident, $pin: ident, $pf: expr) => {
        impl crate::i2c::SdaPin<crate::peripherals::$instance> for crate::peripherals::$pin {
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
    fn timer_period_32mhz() {
        assert_eq!(timer_period(32_000_000, 100_000), Ok(31));
        assert_eq!(timer_period(32_000_000, 400_000), Ok(7));
        // 1 MHz is not reachable from 32 MHz: 800 kHz is the closest below.
        assert_eq!(timer_period(32_000_000, 1_000_000), Ok(3));
        assert_eq!(timer_period(32_000_000, 0), Err(ConfigError::InvalidFrequency));
        assert_eq!(timer_period(32_000_000, 10_000), Err(ConfigError::InvalidFrequency));
    }
}
