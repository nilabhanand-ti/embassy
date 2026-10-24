//! Blinks the user LEDs of the LP-AM13E232.

#![no_std]
#![no_main]

use embassy_am13::Config;
use embassy_am13::gpio::{Level, Output};
use embassy_executor::Spawner;
use embassy_time::Timer;
use panic_halt as _;

#[embassy_executor::main]
async fn main(_spawner: Spawner) -> ! {
    let p = embassy_am13::init(Config::default());

    // USER_LED0 (red) on PD0 and USER_LED1 (green) on PD1, both active low.
    let mut red = Output::new(p.PD0, Level::High);
    let mut green = Output::new(p.PD1, Level::Low);

    loop {
        Timer::after_millis(500).await;

        red.toggle();
        green.toggle();
    }
}
