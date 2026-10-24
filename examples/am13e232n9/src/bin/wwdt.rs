//! Example of using the window watchdog timer.
//!
//! The watchdog is pet five times, then no more, so it resets the device. After the reset the
//! example reports the reset cause (a watchdog violation) on the backchannel UART (UC4 on PA0 and
//! PA1, 115200 baud) and starts over.

#![no_std]
#![no_main]

use core::fmt::Write;

use embassy_am13::uart::{self, UartTx};
use embassy_am13::wwdt::{ClosedWindowPercentage, Config, Timeout, Watchdog};
use embassy_executor::Spawner;
use embassy_time::Timer;
use panic_halt as _;

#[embassy_executor::main]
async fn main(_spawner: Spawner) -> ! {
    let p = embassy_am13::init(Default::default());

    let mut uart = UartTx::new_blocking(p.UC4, p.PA0, uart::Config::default()).unwrap();

    let mut line = heapless::String::<64>::new();
    let _ = write!(line, "reset cause: {:?}\r\n", embassy_am13::read_reset_cause());
    uart.blocking_write(line.as_bytes()).unwrap();

    let mut conf = Config::default();
    conf.timeout = Timeout::Sec1;
    // The watchdog also resets the device if it is pet less than 250 ms (25% of 1 s) after the
    // previous pet.
    conf.closed_window = ClosedWindowPercentage::TwentyFive;
    let mut wdt = Watchdog::new(p.WWDT0, conf);

    Timer::after_millis(900).await;

    for _ in 1..=5 {
        uart.blocking_write(b"pet\r\n").unwrap();
        wdt.pet();
        Timer::after_millis(500).await;
    }

    uart.blocking_write(b"no more pets, expect a reset\r\n").unwrap();

    loop {
        Timer::after_millis(100).await;
    }
}
