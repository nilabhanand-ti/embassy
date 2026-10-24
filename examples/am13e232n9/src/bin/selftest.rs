//! On-target self test of the HAL on an LP-AM13E232, needing no external wiring.
//!
//! Results are printed on the backchannel UART (UC4 on PA0 and PA1, 115200 baud), one line per
//! check, ending in `selftest: <passed> passed, <failed> failed`. `hil_test.py` runs it.
//!
//! - DMA: software triggered transfers of every width, blocking and async (the transfers of
//!   `tests/mspm0/src/bin/dma.rs`).
//! - GPIO edge interrupts on port D: a pin is configured as both input and output, and its own
//!   output is toggled while waiting for the edge. The pins are the LED pins, PD0 and PD1.
//! - SPI (UC0, PD7/PD8/PD6) in its internal loopback mode, 8 and 16-bit words, blocking and
//!   async.
//! - I2C (UC2 on PA23 and PA22, with the internal pull-ups): the bus idles high, and an address no
//!   target answers is not acknowledged (blocking and async).
//! - ADC0: VSSA, VREFLO and VDDA through the internal channels, VDDCORE and the temperature
//!   sensor within plausible ranges.
//! - PWM on TIMG12 channel 1 (PA27): the duty cycle and frequency, measured by sampling the pad.
//! - Time driver: `Timer::after_millis(100)` measured with the CPU cycle counter (MCLK is 32 MHz).

#![no_std]
#![no_main]

use core::fmt::Write as _;
use core::slice;

use embassy_am13::adc::{self, Adc, TestSignal};
use embassy_am13::dma::{self, Channel, Transfer, TransferMode, TransferOptions, Word};
use embassy_am13::gpio::{Flex, Level, Pull};
use embassy_am13::i2c::{self, I2c};
use embassy_am13::spi::{self, Spi};
use embassy_am13::tim::simple_pwm::{self, PwmPin, SimplePwm};
use embassy_am13::uart::{self, UartTx};
use embassy_am13::{Peri, bind_interrupts, pac, peripherals};
use embassy_executor::Spawner;
use embassy_futures::join::join;
use embassy_time::{Duration, Timer, with_timeout};
use panic_halt as _;

bind_interrupts!(
    struct Irqs {
        DMA => dma::InterruptHandler<peripherals::DMA_CH0>;
        UC0 => spi::InterruptHandler<peripherals::UC0>;
        UC2 => i2c::InterruptHandler<peripherals::UC2>;
        ADC0_INT1 => adc::InterruptHandler<peripherals::ADC0>;
    }
);

struct Report<'d> {
    uart: UartTx<'d, embassy_am13::mode::Blocking>,
    passed: u32,
    failed: u32,
}

impl Report<'_> {
    fn check(&mut self, name: &str, ok: bool) {
        if ok {
            self.passed += 1;
        } else {
            self.failed += 1;
        }
        self.line(format_args!("{} {}", if ok { "ok" } else { "FAIL" }, name));
    }

    fn line(&mut self, args: core::fmt::Arguments) {
        let mut s = heapless::String::<192>::new();
        let _ = s.write_fmt(args);
        let _ = s.push_str("\r\n");
        self.uart.blocking_write(s.as_bytes()).unwrap();
    }
}

#[embassy_executor::main]
async fn main(_spawner: Spawner) -> ! {
    let mut p = embassy_am13::init(Default::default());

    let uart = UartTx::new_blocking(p.UC4, p.PA0, uart::Config::default()).unwrap();
    let mut r = Report {
        uart,
        passed: 0,
        failed: 0,
    };

    r.line(format_args!("selftest: start"));
    r.line(format_args!("reset cause: {:?}", embassy_am13::read_reset_cause()));

    // DMA
    let mut ch = Channel::new(p.DMA_CH0, Irqs);

    r.check("dma u8 read", single_read(ch.reborrow(), 0x41_u8));
    r.check("dma u16 read", single_read(ch.reborrow(), 0xFF41_u16));
    r.check("dma u32 read", single_read(ch.reborrow(), 0xFFEE_FF41_u32));
    r.check("dma u64 read", single_read(ch.reborrow(), 0x0011_2233_FFEE_FF41_u64));
    r.check("dma u8 read async", async_single_read(ch.reborrow(), 0x42_u8).await);
    r.check(
        "dma u32 read async",
        async_single_read(ch.reborrow(), 0xFE44_1500_u32).await,
    );
    r.check("dma u8 block read", block_read::<_, 16>(ch.reborrow(), 0x98_u8));
    r.check(
        "dma u64 block read",
        block_read::<_, 4>(ch.reborrow(), 0xABCD_EF01_2345_6789_u64),
    );
    r.check(
        "dma u16 block read async",
        async_block_read::<_, 6>(ch.reborrow(), 0x7777_u16).await,
    );
    r.check(
        "dma u8 block write",
        block_write(ch.reborrow(), &[0xFF_u8, 0x7F, 0x3F, 0x1F]),
    );
    r.check(
        "dma u32 block write",
        block_write(ch.reborrow(), &[0xFF00_00FF_u32, 0xFF00_007F, 0x0000_FF3F, 0xFF1F_0000]),
    );
    r.check("dma u8 widening read", widening_read::<u8, u32>(ch.reborrow(), 0x43));

    // GPIO edge interrupts on port D, on both LED pins.
    let mut pd0 = p.PD0;
    let mut pd1 = p.PD1;
    r.check(
        "gpio PD0 falling edge",
        edge_loopback(pd0.reborrow().into(), 0, Port::D, false).await,
    );
    r.check(
        "gpio PD0 rising edge",
        edge_loopback(pd0.reborrow().into(), 0, Port::D, true).await,
    );
    r.check(
        "gpio PD1 falling edge",
        edge_loopback(pd1.reborrow().into(), 1, Port::D, false).await,
    );
    r.check(
        "gpio PD1 rising edge",
        edge_loopback(pd1.reborrow().into(), 1, Port::D, true).await,
    );

    // SPI in loopback mode: what is sent comes back.
    {
        let mut config = spi::Config::default();
        config.frequency = 1_000_000;
        let mut spi = Spi::new(
            p.UC0.reborrow(),
            p.PD7.reborrow(),
            p.PD8.reborrow(),
            p.PD6.reborrow(),
            Irqs,
            config,
        )
        .unwrap();
        spi.set_loopback(true);

        let tx: [u8; 16] = core::array::from_fn(|i| (i as u8).wrapping_mul(37) ^ 0xA5);
        let mut rx = [0u8; 16];
        spi.blocking_transfer(&mut rx, &tx).unwrap();
        r.check("spi u8 loopback", rx == tx);

        let tx16: [u16; 8] = core::array::from_fn(|i| 0x1234u16.wrapping_mul(i as u16 + 1));
        let mut rx16 = [0u16; 8];
        spi.blocking_transfer(&mut rx16, &tx16).unwrap();
        r.check("spi u16 loopback", rx16 == tx16);

        let mut words = tx;
        spi.transfer_in_place(&mut words).await.unwrap();
        r.check("spi u8 loopback async", words == tx);

        config.frequency = 8_000_000;
        spi.set_config(&config).unwrap();
        let mut rx = [0u8; 16];
        spi.transfer(&mut rx, &tx).await.unwrap();
        r.check("spi 8 MHz loopback async", rx == tx);
    }

    // I2C: nobody answers on these pins, so every address is not acknowledged.
    {
        let mut config = i2c::Config::default();
        config.sda_pull = Pull::Up;
        config.scl_pull = Pull::Up;
        let mut bus = I2c::new(p.UC2.reborrow(), p.PA23.reborrow(), p.PA22.reborrow(), Irqs, config).unwrap();

        r.check("i2c bus idle high", bus.bus_levels() == (true, true));
        r.check(
            "i2c write nack",
            bus.blocking_write(0x2A, &[1, 2, 3]) == Err(i2c::Error::AddressNack),
        );
        let mut buf = [0u8; 4];
        r.check(
            "i2c read nack",
            bus.blocking_read(0x2A, &mut buf) == Err(i2c::Error::AddressNack),
        );
        r.check(
            "i2c write_read nack async",
            bus.write_read(0x51, &[0], &mut buf).await == Err(i2c::Error::AddressNack),
        );
        r.check("i2c bus released", bus.bus_levels() == (true, true));
    }

    // ADC0 on its internal channels, with the internal 3.3 V reference.
    {
        let mut adc = Adc::new(p.ADC0.reborrow(), Irqs, adc::Config::default());

        let vssa = adc.blocking_read(&mut adc::Channel::internal_test(TestSignal::Vssa));
        let vreflo = adc.blocking_read(&mut adc::Channel::vreflo());
        let vdda = adc.blocking_read(&mut adc::Channel::internal_test(TestSignal::Vdda));
        let vddcore = adc.read(&mut adc::Channel::internal_test(TestSignal::VddCore)).await;
        let temperature = adc.read(&mut adc::Channel::temperature_sensor()).await;
        let calibration = adc::temperature_calibration();

        r.line(format_args!(
            "adc: VSSA {} VREFLO {} VDDA {} VDDCORE {} ({} mV) temperature {}",
            vssa,
            vreflo,
            vdda,
            vddcore,
            vddcore as u32 * 3300 / 4095,
            temperature
        ));
        r.line(format_args!("adc: temperature calibration {:?}", calibration));
        r.check("adc vssa", vssa < 100);
        r.check("adc vreflo", vreflo < 100);
        // VDDA is the 3.3 V reference itself.
        r.check("adc vdda", vdda > 3900);
        // VDDCORE is about 1.2 V.
        r.check("adc vddcore async", (1000..2500).contains(&vddcore));
        match calibration.celsius(temperature) {
            Some(celsius) => {
                r.line(format_args!("adc: temperature {} C", celsius as i32));
                r.check("adc temperature", (0.0..70.0).contains(&celsius));
            }
            None => r.check("adc temperature (uncalibrated)", (200..3900).contains(&temperature)),
        }
    }

    // PWM on TIMG12 channel 1, 1 kHz at 25%, sampled on the pad.
    {
        let mut config = simple_pwm::Config::default();
        config.frequency = 1_000;
        let mut pwm = SimplePwm::new_2ch_32bit(
            p.TIMG12.reborrow(),
            None,
            Some(PwmPin::new(p.PA27.reborrow(), Pull::None, false)),
            config,
        )
        .unwrap();
        let mut ch = pwm.ch1();
        let max = ch.max_duty_cycle();
        ch.set_duty_cycle(max / 4).unwrap();
        ch.enable();
        // A new duty cycle takes effect at the end of the running period.
        Timer::after_millis(3).await;

        // The pad's input buffer lets GPIO read the pin while the timer drives it. Port A pin n
        // is PINCM n on the AM13E2x.
        pac::IOMUX.pincm(27).modify(|w| w.set_inena(true));

        let mut cp = unsafe { cortex_m::Peripherals::steal() };
        cp.DCB.enable_trace();
        cp.DWT.enable_cycle_counter();
        let (mut high, mut total, mut rising, mut last) = (0u32, 0u32, 0u32, false);
        let start = cortex_m::peripheral::DWT::cycle_count();
        // 20 ms: 20 periods.
        while cortex_m::peripheral::DWT::cycle_count().wrapping_sub(start) < 640_000 {
            let level = pac::GPIOA.din31_0().read().dio(27);
            high += level as u32;
            total += 1;
            rising += (level && !last) as u32;
            last = level;
        }
        let duty = high * 1000 / total;
        r.line(format_args!(
            "pwm: {} Hz, max duty {}, measured {}.{}% high, {} rising edges in 20 ms",
            ch.get_frequency(),
            max,
            duty / 10,
            duty % 10,
            rising
        ));
        r.check("pwm frequency", ch.get_frequency() == 1_000);
        r.check("pwm duty 25%", (230..270).contains(&duty));
        r.check("pwm period", (19..=21).contains(&rising));
    }

    // Time driver against the cycle counter.
    let mut cp = unsafe { cortex_m::Peripherals::steal() };
    cp.DCB.enable_trace();
    cp.DWT.enable_cycle_counter();
    let start = cortex_m::peripheral::DWT::cycle_count();
    Timer::after_millis(100).await;
    let cycles = cortex_m::peripheral::DWT::cycle_count().wrapping_sub(start);
    // The time driver counts LFCLK, which runs from LFOSC: 32.768 kHz +-3% (+-5% above 85 C). The
    // cycle counter runs from SYSOSC: 32 MHz +-1%. So 3.2 million cycles, give or take 4%.
    r.line(format_args!(
        "timer: 100 ms took {} cycles, LFCLK ~{} Hz",
        cycles,
        // 3276.8 LFCLK ticks in `cycles / 32 MHz` seconds.
        104_857_600_000u64 / cycles as u64
    ));
    r.check("timer 100 ms", (3_070_000..3_340_000).contains(&cycles));

    let (passed, failed) = (r.passed, r.failed);
    r.line(format_args!("selftest: {} passed, {} failed", passed, failed));

    loop {
        Timer::after_secs(1).await;
    }
}

#[derive(Clone, Copy)]
enum Port {
    D,
}

/// Wait for an edge on a pin while driving that edge on the same pin.
async fn edge_loopback(pin: Peri<'_, embassy_am13::gpio::AnyPin>, bit: u32, port: Port, rising: bool) -> bool {
    let mut flex = Flex::new(pin);
    flex.set_level(if rising { Level::Low } else { Level::High });
    flex.set_as_input_output();
    Timer::after_millis(1).await;

    let gpio = match port {
        Port::D => pac::GPIOD,
    };

    let wait = async {
        if rising {
            flex.wait_for_rising_edge().await
        } else {
            flex.wait_for_falling_edge().await
        }
    };

    // Flip the output from outside the borrow of `flex`, after the wait has armed the interrupt.
    let drive = async {
        Timer::after_millis(5).await;
        if rising {
            gpio.doutset31_0().write(|w| w.0 = 1 << bit);
        } else {
            gpio.doutclr31_0().write(|w| w.0 = 1 << bit);
        }
    };

    with_timeout(Duration::from_millis(100), join(wait, drive))
        .await
        .is_ok()
}

fn single_read<W: Word + Copy + Default + Eq>(mut channel: Channel<'_>, mut src: W) -> bool {
    let options = TransferOptions::default();
    let mut dst = W::default();

    // SAFETY: src and dst outlive the transfer.
    let transfer =
        unsafe { channel.read(Transfer::SOFTWARE_TRIGGER, &mut src, slice::from_mut(&mut dst), options) }.unwrap();
    transfer.blocking_wait();

    src == dst
}

async fn async_single_read<W: Word + Copy + Default + Eq>(mut channel: Channel<'_>, mut src: W) -> bool {
    let options = TransferOptions::default();
    let mut dst = W::default();

    // SAFETY: src and dst outlive the transfer.
    let transfer =
        unsafe { channel.read(Transfer::SOFTWARE_TRIGGER, &mut src, slice::from_mut(&mut dst), options) }.unwrap();
    transfer.await;

    src == dst
}

fn block_read<W: Word + Copy + Default + Eq, const N: usize>(mut channel: Channel<'_>, mut src: W) -> bool {
    let mut options = TransferOptions::default();
    options.mode = TransferMode::Block;

    let mut dst = [W::default(); N];

    // SAFETY: src and dst outlive the transfer.
    let transfer = unsafe { channel.read(Transfer::SOFTWARE_TRIGGER, &mut src, &mut dst[..], options) }.unwrap();
    transfer.blocking_wait();

    dst == [src; N]
}

async fn async_block_read<W: Word + Copy + Default + Eq, const N: usize>(mut channel: Channel<'_>, mut src: W) -> bool {
    let mut options = TransferOptions::default();
    options.mode = TransferMode::Block;

    let mut dst = [W::default(); N];

    // SAFETY: src and dst outlive the transfer.
    let transfer = unsafe { channel.read(Transfer::SOFTWARE_TRIGGER, &mut src, &mut dst[..], options) }.unwrap();
    transfer.await;

    dst == [src; N]
}

fn block_write<W: Word + Copy + Default + Eq>(mut channel: Channel<'_>, src: &[W]) -> bool {
    let mut options = TransferOptions::default();
    options.mode = TransferMode::Block;

    let mut dst = W::default();

    // Starting from 1 because a zero length transfer does nothing.
    for i in 1..src.len() {
        // SAFETY: src and dst outlive the transfer.
        let transfer = unsafe { channel.write(Transfer::SOFTWARE_TRIGGER, &src[..i], &mut dst, options) }.unwrap();
        transfer.blocking_wait();

        // The result is the last value written.
        if dst != src[i - 1] {
            return false;
        }
    }

    true
}

/// The DMA zeroes the upper bytes when the destination is wider than the source.
fn widening_read<SW, DW>(mut channel: Channel<'_>, mut src: SW) -> bool
where
    SW: Word + Copy + Default + Eq,
    DW: Word + Copy + Default + Eq + From<SW>,
{
    let options = TransferOptions::default();
    let mut dst = DW::default();

    // SAFETY: src and dst outlive the transfer.
    let transfer =
        unsafe { channel.read(Transfer::SOFTWARE_TRIGGER, &mut src, slice::from_mut(&mut dst), options) }.unwrap();
    transfer.blocking_wait();

    DW::from(src) == dst
}
