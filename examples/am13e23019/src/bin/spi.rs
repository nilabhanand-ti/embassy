//! SPI controller on UC3: SCLK on PB5, PICO on PB10, POCI on PA20, the pins of the SDK's SPI
//! examples on the LaunchPad.
//!
//! Exchanges a few bytes once a second and prints what was received on the backchannel UART (UC4
//! on PA0 and PA1, 115200 baud). Connect PB10 to PA20 to receive what is sent, or leave them open
//! to read the POCI line.

#![no_std]
#![no_main]

use core::fmt::Write;

use embassy_am13::spi::{self, Spi};
use embassy_am13::uart::{self, UartTx};
use embassy_am13::{Config, bind_interrupts, peripherals};
use embassy_executor::Spawner;
use embassy_time::Timer;
use panic_halt as _;

bind_interrupts!(struct Irqs {
    UC3 => spi::InterruptHandler<peripherals::UC3>;
});

#[embassy_executor::main]
async fn main(_spawner: Spawner) -> ! {
    let p = embassy_am13::init(Config::default());

    let mut uart = UartTx::new_blocking(p.UC4, p.PA0, uart::Config::default()).unwrap();

    let mut config = spi::Config::default();
    config.frequency = 1_000_000;
    let mut spi = Spi::new(p.UC3, p.PB5, p.PB10, p.PA20, Irqs, config).unwrap();

    let mut counter = 0u8;
    loop {
        let tx = [0xA5, counter, !counter, 0x5A];
        let mut rx = [0u8; 4];
        spi.transfer(&mut rx, &tx).await.unwrap();

        let mut line = heapless::String::<64>::new();
        let _ = write!(line, "sent {tx:02x?}, received {rx:02x?}\r\n");
        uart.blocking_write(line.as_bytes()).unwrap();

        counter = counter.wrapping_add(1);
        Timer::after_secs(1).await;
    }
}
