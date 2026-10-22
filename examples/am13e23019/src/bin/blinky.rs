//! Blinks the user LEDs of the LP-AM13E230.

#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_mspm0::Config;
use embassy_mspm0::gpio::{Level, Output};
use embassy_time::Timer;
use panic_halt as _;

#[embassy_executor::main]
async fn main(_spawner: Spawner) -> ! {
    let p = embassy_mspm0::init(Config::default());

    // LED4 (red) on PA5 and LED5 (green) on PB0, both active low.
    let mut red = Output::new(p.PA5, Level::High);
    let mut green = Output::new(p.PB0, Level::Low);

    loop {
        Timer::after_millis(500).await;

        red.toggle();
        green.toggle();
    }
}
