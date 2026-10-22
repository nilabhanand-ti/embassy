//! On-target self test of the HAL on an LP-AM13E232, needing no external wiring.
//!
//! Results are printed on the backchannel UART (UC4 on PA0 and PA1, 115200 baud), one line per
//! check, ending in `selftest: <passed> passed, <failed> failed`. `hil_test.py` runs it.
//!
//! - DMA: software triggered transfers of every width, blocking and async (the transfers of
//!   `tests/mspm0/src/bin/dma.rs`).
//! - GPIO edge interrupts on port D: a pin is configured as both input and output, and its own
//!   output is toggled while waiting for the edge. The pins are the LED pins, PD0 and PD1.
//! - Time driver: `Timer::after_millis(100)` measured with the CPU cycle counter (MCLK is 32 MHz).

#![no_std]
#![no_main]

use core::fmt::Write as _;
use core::slice;

use embassy_executor::Spawner;
use embassy_futures::join::join;
use embassy_mspm0::dma::{self, Channel, Transfer, TransferMode, TransferOptions, Word};
use embassy_mspm0::gpio::{Flex, Level};
use embassy_mspm0::uart::{self, UartTx};
use embassy_mspm0::{Peri, bind_interrupts, pac, peripherals};
use embassy_time::{Duration, Timer, with_timeout};
use panic_halt as _;

bind_interrupts!(
    struct Irqs {
        DMA => dma::InterruptHandler<peripherals::DMA_CH0>;
    }
);

struct Report<'d> {
    uart: UartTx<'d, embassy_mspm0::mode::Blocking>,
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
        let mut s = heapless::String::<96>::new();
        let _ = s.write_fmt(args);
        let _ = s.push_str("\r\n");
        self.uart.blocking_write(s.as_bytes()).unwrap();
    }
}

#[embassy_executor::main]
async fn main(_spawner: Spawner) -> ! {
    let p = embassy_mspm0::init(Default::default());

    let uart = UartTx::new_blocking(p.UC4, p.PA0, uart::Config::default()).unwrap();
    let mut r = Report {
        uart,
        passed: 0,
        failed: 0,
    };

    r.line(format_args!("selftest: start"));
    r.line(format_args!("reset cause: {:?}", embassy_mspm0::read_reset_cause()));

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
async fn edge_loopback(pin: Peri<'_, embassy_mspm0::gpio::AnyPin>, bit: u32, port: Port, rising: bool) -> bool {
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
