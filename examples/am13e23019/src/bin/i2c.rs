//! I2C controller on UC2: SCL on PA23, SDA on PA22 (the pins of the SDK's I2C examples), 100 kHz.
//!
//! Scans the bus once and prints the address of every target which acknowledges on the
//! backchannel UART (UC4 on PA0 and PA1, 115200 baud). The internal pull-ups are enabled; add
//! external ones (2.2 kΩ to 4.7 kΩ) for a real bus.

#![no_std]
#![no_main]

use core::fmt::Write;

use embassy_am13::gpio::Pull;
use embassy_am13::i2c::{self, Error, I2c};
use embassy_am13::uart::{self, UartTx};
use embassy_am13::{Config, bind_interrupts, peripherals};
use embassy_executor::Spawner;
use embassy_time::Timer;
use panic_halt as _;

bind_interrupts!(struct Irqs {
    UC2 => i2c::InterruptHandler<peripherals::UC2>;
});

#[embassy_executor::main]
async fn main(_spawner: Spawner) -> ! {
    let p = embassy_am13::init(Config::default());

    let mut uart = UartTx::new_blocking(p.UC4, p.PA0, uart::Config::default()).unwrap();

    let mut config = i2c::Config::default();
    config.sda_pull = Pull::Up;
    config.scl_pull = Pull::Up;
    let mut i2c = I2c::new(p.UC2, p.PA23, p.PA22, Irqs, config).unwrap();

    let (scl, sda) = i2c.bus_levels();
    let mut line = heapless::String::<64>::new();
    let _ = write!(line, "bus idle: SCL {scl}, SDA {sda}\r\nscanning\r\n");
    uart.blocking_write(line.as_bytes()).unwrap();

    let mut found = 0;
    for address in 0x08..0x78 {
        let mut byte = [0u8];
        match i2c.read(address, &mut byte).await {
            Ok(()) => {
                found += 1;
                let mut line = heapless::String::<32>::new();
                let _ = write!(line, "target at 0x{address:02x}\r\n");
                uart.blocking_write(line.as_bytes()).unwrap();
            }
            Err(Error::AddressNack) => (),
            Err(e) => {
                let mut line = heapless::String::<48>::new();
                let _ = write!(line, "0x{address:02x}: {e:?}\r\n");
                uart.blocking_write(line.as_bytes()).unwrap();
            }
        }
    }

    let mut line = heapless::String::<32>::new();
    let _ = write!(line, "{found} targets found\r\n");
    uart.blocking_write(line.as_bytes()).unwrap();

    loop {
        Timer::after_secs(1).await;
    }
}
