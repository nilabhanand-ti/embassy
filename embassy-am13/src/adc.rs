//! Analog to digital converters (ADC0, ADC1 and ADC2).
//!
//! The AM13E2x ADCs are 12-bit converters with 16 start-of-conversion (SOC) slots, run by four
//! sequencers; the results are in a block of their own (ADCxRESULT). This is not the MSPM0 ADC12.
//!
//! The driver converts one channel at a time, software triggered, with SOC0 and sequencer 1:
//!
//! ```rust,ignore
//! let mut adc = Adc::new_blocking(p.ADC0, Config::default());
//! let mut pin = Channel::pin(p.PB28);
//! let raw = adc.blocking_read(&mut pin);
//! ```

#![macro_use]

use core::future::poll_fn;
use core::marker::PhantomData;
use core::task::Poll;

use embassy_hal_internal::PeripheralType;
use embassy_sync::waitqueue::AtomicWaker;

use crate::gpio::{AnyPin, SealedPin};
use crate::interrupt::typelevel::{Binding, Interrupt as _};
use crate::mode::{Async, Blocking, Mode};
use crate::pac::adc::Adc as Regs;
use crate::pac::adcresult::Adcresult as ResultRegs;
use crate::{Peri, pac, sysctl};

/// The conversion range: the reference of all three ADCs (SYSCTL.ANAREFCTL).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Vref {
    /// Internal reference, 0 V to 3.3 V (VDDA).
    Internal3v3,
    /// Internal reference, 0 V to 2.5 V.
    Internal2v5,
    /// External reference: 1.65 V on VREFHI for a 0 V to 3.3 V range.
    ExternalHalfScale,
    /// External reference: the full range on VREFHI.
    ExternalFullScale,
}

/// ADC configuration.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Config {
    /// The reference. It is shared by the three ADCs: the last ADC created sets it.
    pub vref: Vref,

    /// The sample window, in MCLK cycles (32 MHz while the parts run from SYSOSC), from 1 to
    /// 1472. Values the hardware cannot represent are rounded up.
    ///
    /// The window must be at least 40 ns. The default, 64 cycles (2 µs), suits sources of a few
    /// kΩ and the internal channels.
    pub sample_cycles: u16,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            vref: Vref::Internal3v3,
            sample_cycles: 64,
        }
    }
}

/// The source of the internal test channel (SYSCTL.INTERNALTESTCTL.TESTSEL).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum TestSignal {
    /// The core supply, VDDCORE.
    VddCore,
    /// The analog supply, VDDA.
    Vdda,
    /// The analog ground, VSSA.
    Vssa,
    /// The low reference, VREFLO.
    VrefLo,
}

impl TestSignal {
    fn testsel(self) -> u8 {
        match self {
            Self::VddCore => 1,
            Self::Vdda => 2,
            Self::Vssa => 3,
            Self::VrefLo => 4,
        }
    }
}

/// CHSEL of VREFLO, on every ADC.
const CHSEL_VREFLO: u8 = 0;
/// CHSEL of the temperature sensor, on ADC0.
const CHSEL_TEMPERATURE: u8 = 13;
/// CHSEL of the internal test channel, on every ADC.
const CHSEL_INTERNAL_TEST: u8 = 20;

/// An input of ADC `T`: a pin or an internal signal.
pub struct Channel<'d, T: Instance> {
    chsel: u8,
    test: Option<TestSignal>,
    _pin: Option<Peri<'d, AnyPin>>,
    _phantom: PhantomData<T>,
}

impl<'d, T: Instance> Channel<'d, T> {
    /// An analog input pin. The pin is disconnected from its digital functions while the
    /// channel exists.
    pub fn pin(pin: Peri<'d, impl AdcPin<T>>) -> Self {
        let chsel = pin.channel();
        let pin: Peri<'d, AnyPin> = pin.into();
        pin.set_as_analog();

        Self {
            chsel,
            test: None,
            _pin: Some(pin),
            _phantom: PhantomData,
        }
    }

    /// VREFLO, the low reference: reads close to 0.
    pub fn vreflo() -> Self {
        Self {
            chsel: CHSEL_VREFLO,
            test: None,
            _pin: None,
            _phantom: PhantomData,
        }
    }

    /// The internal test channel, connected to `signal` while it is converted.
    ///
    /// The test multiplexer is shared by the three ADCs.
    pub fn internal_test(signal: TestSignal) -> Self {
        Self {
            chsel: CHSEL_INTERNAL_TEST,
            test: Some(signal),
            _pin: None,
            _phantom: PhantomData,
        }
    }

    /// The channel number (CHSEL).
    pub fn channel(&self) -> u8 {
        self.chsel
    }
}

impl<'d> Channel<'d, crate::peripherals::ADC0> {
    /// The internal temperature sensor, which ADC0 converts. Creating the channel powers the sensor
    /// up (SYSCTL.TSNSSCTL); it stays on.
    ///
    /// See [`temperature_calibration`] to convert the readings.
    pub fn temperature_sensor() -> Self {
        pac::SYSCTL.tsnssctl().modify(|w| w.set_enable(true));

        Self {
            chsel: CHSEL_TEMPERATURE,
            test: None,
            _pin: None,
            _phantom: PhantomData,
        }
    }
}

/// The factory calibration of the temperature sensor, for the internal 3.3 V reference: the
/// reading at two test temperatures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct TemperatureCalibration {
    /// The reading at `low_temperature`.
    pub low_code: u16,
    /// The lower test temperature, nominally 30 °C, as recorded by the factory.
    pub low_temperature: u16,
    /// The reading at `high_temperature`.
    pub high_code: u16,
    /// The higher test temperature, nominally 125 °C, as recorded by the factory.
    pub high_temperature: u16,
}

impl TemperatureCalibration {
    /// The temperature in °C of a reading of the temperature sensor, interpolated between the
    /// calibration points; `None` if the calibration is missing (erased) or unusable.
    pub fn celsius(&self, code: u16) -> Option<f32> {
        let blank = |v: u16| v == 0 || v == 0xFFFF;
        if [
            self.low_code,
            self.high_code,
            self.low_temperature,
            self.high_temperature,
        ]
        .into_iter()
        .any(blank)
            || self.low_code == self.high_code
        {
            return None;
        }

        let slope = (self.high_temperature as f32 - self.low_temperature as f32)
            / (self.high_code as f32 - self.low_code as f32);
        Some(self.low_temperature as f32 + (code as f32 - self.low_code as f32) * slope)
    }
}

/// Read the factory calibration of the temperature sensor (FACTORYREGION TSCODE_MP1,
/// TSCODE_MP3 and TSTEMP; hw_factoryregion.h).
pub fn temperature_calibration() -> TemperatureCalibration {
    const TSCODE_MP1: *const u32 = 0x6011_1068 as *const u32;
    const TSCODE_MP3: *const u32 = 0x6011_106C as *const u32;
    const TSTEMP: *const u32 = 0x6011_1070 as *const u32;

    // SAFETY: read-only words of the factory region.
    let (mp1, mp3, temp) = unsafe {
        (
            TSCODE_MP1.read_volatile(),
            TSCODE_MP3.read_volatile(),
            TSTEMP.read_volatile(),
        )
    };

    TemperatureCalibration {
        // The internal reference codes are the upper halves.
        low_code: (mp1 >> 16) as u16,
        low_temperature: temp as u16,
        high_code: (mp3 >> 16) as u16,
        high_temperature: (temp >> 16) as u16,
    }
}

/// ADC driver.
pub struct Adc<'d, T: Instance, M: Mode> {
    _phantom: PhantomData<(&'d mut T, M)>,
}

impl<'d, T: Instance> Adc<'d, T, Blocking> {
    /// Create a blocking ADC driver.
    ///
    /// Powering the converter up takes 5 ms, which this waits for.
    pub fn new_blocking(peri: Peri<'d, T>, config: Config) -> Self {
        Self::new_inner(peri, config)
    }
}

impl<'d, T: Instance> Adc<'d, T, Async> {
    /// Create an interrupt driven ADC driver.
    ///
    /// Powering the converter up takes 5 ms, which this waits for (blocking).
    pub fn new(peri: Peri<'d, T>, _irq: impl Binding<T::Interrupt, InterruptHandler<T>> + 'd, config: Config) -> Self {
        let this = Self::new_inner(peri, config);
        T::Interrupt::unpend();
        this
    }

    /// Convert a channel.
    pub async fn read(&mut self, channel: &mut Channel<'_, T>) -> u16 {
        let r = T::regs();

        self.start(channel);

        poll_fn(|cx| {
            T::state().waker.register(cx.waker());

            if conversion_done(r) {
                return Poll::Ready(());
            }

            // The interrupt handler disables the interrupt again and wakes the task.
            unsafe { T::Interrupt::enable() };
            if conversion_done(r) {
                T::Interrupt::disable();
                return Poll::Ready(());
            }

            Poll::Pending
        })
        .await;

        self.result()
    }
}

impl<'d, T: Instance, M: Mode> Adc<'d, T, M> {
    fn new_inner(_peri: Peri<'d, T>, config: Config) -> Self {
        let r = T::regs();

        r.gprcm().rstctl().write(|w| {
            w.set_resetstkyclr(true);
            w.set_resetassert(true);
            w.set_key(pac::adc::vals::RstctlKey::Key);
        });
        r.gprcm().pwren().write(|w| {
            w.set_enable(true);
            w.set_key(pac::adc::vals::PwrenKey::Key);
        });
        cortex_m::asm::delay(16);

        set_vref(config.vref);

        // ADCCLK is MCLK, at most 125 MHz: no division is needed while MCLK is 32 MHz.
        r.adcctl2().write(|w| w.set_prescale(0));

        // The interrupt flag rises at the end of the conversion, once the result is latched.
        r.adcctl1().write(|w| {
            w.set_intpulsepos(true);
            w.set_adcpwdnz(true);
        });

        // SOC0, run alone by sequencer 1 on a software trigger; ADCINT1 at its end.
        r.adcseqctl().write(|w| w.set_seqend(0));
        r.adcseqconfig(0).write(|w| {
            w.set_acqps(acqps(config.sample_cycles));
            w.set_seqstart(0);
            w.set_trigsel(0);
            w.set_seqenable(true);
        });
        r.adcintsel().write(|w| {
            w.set_intsel(0, 0);
            w.set_intcont(0, false);
            w.set_inte(0, true);
        });
        r.adcintflgclr().write(|w| w.set_adcint(0, true));

        // The converter needs at least 500 µs after ADCPWDNZ; the SDK examples wait 5 ms for the
        // reference to settle.
        cortex_m::asm::delay(sysctl::mclk_frequency() / 200);

        Self { _phantom: PhantomData }
    }

    /// Convert a channel.
    pub fn blocking_read(&mut self, channel: &mut Channel<'_, T>) -> u16 {
        self.start(channel);
        while !conversion_done(T::regs()) {}
        self.result()
    }

    fn start(&mut self, channel: &mut Channel<'_, T>) {
        let r = T::regs();

        if let Some(signal) = channel.test {
            pac::SYSCTL
                .internaltestctl()
                .modify(|w| w.set_testsel(signal.testsel()));
        }

        r.adcsocctl(0).write(|w| w.set_chsel(channel.chsel));
        r.adcintflgclr().write(|w| w.set_adcint(0, true));
        r.adcseqconfig(0).modify(|w| w.set_seqswfrc(true));
    }

    fn result(&mut self) -> u16 {
        let value = T::result_regs().adcresult(0).read();
        T::regs().adcintflgclr().write(|w| w.set_adcint(0, true));
        value
    }
}

impl<'d, T: Instance, M: Mode> Drop for Adc<'d, T, M> {
    fn drop(&mut self) {
        T::Interrupt::disable();
        T::regs().adcctl1().modify(|w| w.set_adcpwdnz(false));
    }
}

fn conversion_done(r: Regs) -> bool {
    let flags = r.adcintflg().read();
    flags.adcint(0) && flags.adcintresult(0)
}

fn set_vref(vref: Vref) {
    let (external, two_five) = match vref {
        Vref::Internal3v3 => (false, false),
        Vref::Internal2v5 => (false, true),
        Vref::ExternalHalfScale => (true, false),
        Vref::ExternalFullScale => (true, true),
    };

    pac::SYSCTL.anarefctl().modify(|w| {
        w.set_anarefsel(external);
        w.set_anaref2p5sel(two_five);
    });
}

/// The ACQPS encoding of a sample window of `cycles` MCLK cycles (dl_adc.h,
/// `DL_ADC_setupSequencer`):
///
/// - `00nnnnnn`: n + 1 cycles (1 to 64)
/// - `01nnnnnn`: 64 + 2 (n + 1) cycles (66 to 192)
/// - `10nnnnnn`: 192 + 4 (n + 1) cycles (196 to 448)
/// - `11nnnnnn`: 448 + 16 (n + 1) cycles (464 to 1472)
fn acqps(cycles: u16) -> u8 {
    let cycles = cycles.clamp(1, 1472);

    let (range, base, step) = match cycles {
        0..=64 => (0u8, 0u16, 1u16),
        65..=192 => (1, 64, 2),
        193..=448 => (2, 192, 4),
        _ => (3, 448, 16),
    };

    let n = (cycles - base).div_ceil(step) - 1;
    (range << 6) | n as u8
}

/// ADC interrupt handler (the ADCx_INT1 interrupt).
pub struct InterruptHandler<T: Instance> {
    _phantom: PhantomData<T>,
}

impl<T: Instance> crate::interrupt::typelevel::Handler<T::Interrupt> for InterruptHandler<T> {
    unsafe fn on_interrupt() {
        T::Interrupt::disable();
        T::state().waker.wake();
    }
}

pub(crate) struct State {
    waker: AtomicWaker,
}

impl State {
    pub(crate) const fn new() -> Self {
        Self {
            waker: AtomicWaker::new(),
        }
    }
}

pub(crate) trait SealedInstance {
    fn regs() -> Regs;
    fn result_regs() -> ResultRegs;
    fn state() -> &'static State;
}

/// An ADC.
#[allow(private_bounds)]
pub trait Instance: SealedInstance + PeripheralType + 'static {
    /// The interrupt the driver uses, ADCx_INT1.
    type Interrupt: crate::interrupt::typelevel::Interrupt;
}

/// An analog input pin of ADC `T`.
pub trait AdcPin<T: Instance>: crate::gpio::Pin {
    /// The channel number (CHSEL) of the pin.
    fn channel(&self) -> u8;
}

macro_rules! impl_adc_instance {
    ($instance: ident, $result: ident, $irq: ident) => {
        impl crate::adc::SealedInstance for crate::peripherals::$instance {
            fn regs() -> crate::pac::adc::Adc {
                crate::pac::$instance
            }

            fn result_regs() -> crate::pac::adcresult::Adcresult {
                crate::pac::$result
            }

            fn state() -> &'static crate::adc::State {
                static STATE: crate::adc::State = crate::adc::State::new();
                &STATE
            }
        }

        impl crate::adc::Instance for crate::peripherals::$instance {
            type Interrupt = crate::interrupt::typelevel::$irq;
        }
    };
}

macro_rules! impl_adc_pin {
    ($instance: ident, $pin: ident, $channel: expr) => {
        impl crate::adc::AdcPin<crate::peripherals::$instance> for crate::peripherals::$pin {
            fn channel(&self) -> u8 {
                $channel
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acqps_encoding() {
        assert_eq!(acqps(1), 0x00);
        assert_eq!(acqps(19), 18);
        assert_eq!(acqps(64), 0x3F);
        // 66 cycles is the first step of the second range; 65 rounds up to it.
        assert_eq!(acqps(65), 0x40);
        assert_eq!(acqps(66), 0x40);
        assert_eq!(acqps(192), 0x7F);
        assert_eq!(acqps(196), 0x80);
        assert_eq!(acqps(448), 0xBF);
        assert_eq!(acqps(464), 0xC0);
        assert_eq!(acqps(1472), 0xFF);
        assert_eq!(acqps(5000), 0xFF);
    }
}
