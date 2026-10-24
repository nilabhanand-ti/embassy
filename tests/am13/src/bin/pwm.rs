#![no_std]
#![no_main]

#[cfg(feature = "am13e232n9")]
teleprobe_meta::target!(b"lp-am13e232");

#[cfg(feature = "am13e23019")]
teleprobe_meta::target!(b"lp-am13e230");

use defmt::{assert, assert_eq, unwrap, *};
use defmt_rtt as _;
use embassy_am13::gpio::Pull;
use embassy_am13::pac;
use embassy_am13::tim::simple_pwm::{self, PwmPin, SimplePwm};
use embassy_executor::Spawner;
use embassy_time::Timer;
use panic_probe as _;

/// Sample PA27 for `cycles` CPU cycles: the share of samples high (per mille) and the rising
/// edges.
fn sample(cycles: u32) -> (u32, u32) {
    let (mut high, mut total, mut rising, mut last) = (0u32, 0u32, 0u32, false);
    let start = cortex_m::peripheral::DWT::cycle_count();
    while cortex_m::peripheral::DWT::cycle_count().wrapping_sub(start) < cycles {
        let level = pac::GPIOA.din31_0().read().dio(27);
        high += level as u32;
        total += 1;
        rising += (level && !last) as u32;
        last = level;
    }
    (high * 1000 / total, rising)
}

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_am13::init(Default::default());
    info!("Hello World!");

    let mut cp = unwrap!(cortex_m::Peripherals::take());
    cp.DCB.enable_trace();
    cp.DWT.enable_cycle_counter();

    // TIMG12 channel 1 on PA27.
    let mut config = simple_pwm::Config::default();
    config.frequency = 1_000;
    let mut pwm = unwrap!(SimplePwm::new_2ch_32bit(
        p.TIMG12,
        None,
        Some(PwmPin::new(p.PA27, Pull::None, false)),
        config
    ));
    // The input buffer of the pad lets GPIO read the pin while the timer drives it.
    pac::IOMUX.pincm(27).modify(|w| w.set_inena(true));

    let mut ch = pwm.ch1();
    ch.enable();
    assert_eq!(ch.get_frequency(), 1_000);
    let max = ch.max_duty_cycle();

    for percent in [10u32, 25, 50, 75, 90] {
        unwrap!(ch.set_duty_cycle(max * percent / 100));
        Timer::after_millis(3).await;
        // 20 ms at 32 MHz: 20 periods.
        let (duty, rising) = sample(640_000);
        info!("{}%: measured {} per mille, {} rising edges", percent, duty, rising);
        assert!(duty.abs_diff(percent * 10) <= 20);
        assert!((19..=21).contains(&rising));
    }

    // A new duty cycle takes effect at the end of the running period.
    ch.set_duty_cycle_fully_off();
    Timer::after_millis(3).await;
    let off = sample(64_000);
    info!("fully off: {} per mille, {} rising edges", off.0, off.1);
    ch.set_duty_cycle_fully_on();
    Timer::after_millis(3).await;
    let on = sample(64_000);
    info!("fully on: {} per mille, {} rising edges", on.0, on.1);
    assert_eq!(off.0, 0);
    assert_eq!(on.0, 1000);

    info!("Test OK");
    cortex_m::asm::bkpt();
}
