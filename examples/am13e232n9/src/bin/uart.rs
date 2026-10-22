//! Example of using blocking uart
//!
//! This uses the XDS110 backchannel UART of the LP-AM13E232 (UC4 on PA0 and PA1), which shows up
//! on the host as the "XDS110 Class Application/User UART" COM port. Open it at 115200 baud.

#![no_std]
#![no_main]

use core::fmt::Write;

use embassy_executor::Spawner;
use embassy_mspm0::uart::{Config, Uart};
use embassy_time::{Instant, Timer};
use panic_halt as _;

#[embassy_executor::main]
async fn main(_spawner: Spawner) -> ! {
    let p = embassy_mspm0::init(Default::default());

    let instance = p.UC4;
    let tx = p.PA0;
    let rx = p.PA1;

    let config = Config::default();
    let mut uart = Uart::new_blocking(instance, rx, tx, config).unwrap();

    uart.blocking_write(b"Hello Embassy World from the AM13E232!\r\n")
        .unwrap();

    // Report the time a few times to show the time driver works.
    for _ in 0..3 {
        let mut line = heapless::String::<64>::new();
        let _ = write!(line, "uptime: {} ms\r\n", Instant::now().as_millis());
        uart.blocking_write(line.as_bytes()).unwrap();
        Timer::after_millis(1000).await;
    }

    uart.blocking_write(b"echoing\r\n").unwrap();

    let mut buf = [0u8; 1];

    loop {
        uart.blocking_read(&mut buf).unwrap();
        uart.blocking_write(&buf).unwrap();
    }
}
