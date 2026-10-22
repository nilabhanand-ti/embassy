//! Example of using buffered uart
//!
//! This uses the XDS110 backchannel UART of the LP-AM13E232 (UC4 on PA0 and PA1). Received bytes
//! are echoed back, and the red LED blinks from another task meanwhile.

#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_mspm0::gpio::{Level, Output};
use embassy_mspm0::uart::{self, BufferedUart, Config};
use embassy_mspm0::{bind_interrupts, peripherals};
use embassy_time::Timer;
use embedded_io_async::Write;
use panic_halt as _;

bind_interrupts!(
    struct Irqs {
        UC4 => uart::BufferedInterruptHandler<peripherals::UC4>;
    }
);

#[embassy_executor::task]
async fn blink(mut led: Output<'static>) {
    loop {
        Timer::after_millis(250).await;
        led.toggle();
    }
}

#[embassy_executor::main]
async fn main(spawner: Spawner) -> ! {
    let p = embassy_mspm0::init(Default::default());

    spawner.spawn(blink(Output::new(p.PD0, Level::High)).unwrap());

    let instance = p.UC4;
    let tx = p.PA0;
    let rx = p.PA1;
    let mut tx_buf = [0u8; 32];
    let mut rx_buf = [0u8; 32];

    let config = Config::default();
    let mut uart = BufferedUart::new(instance, tx, rx, Irqs, &mut tx_buf, &mut rx_buf, config).unwrap();

    uart.write_all(b"Hello Embassy World (buffered)!\r\n").await.unwrap();

    let mut buf = [0u8; 16];

    loop {
        let n = uart.read(&mut buf).await.unwrap();
        uart.write_all(&buf[..n]).await.unwrap();
    }
}
