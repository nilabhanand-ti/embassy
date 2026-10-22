//! Toggles the green LED each time the wake/user button is pressed.
//!
//! The button (MCU_WAKE) is on PB0 and pulls the pin low when pressed. Each press is also reported
//! on the backchannel UART (UC4 on PA0 and PA1, 115200 baud).

#![no_std]
#![no_main]

use core::fmt::Write;

use embassy_executor::Spawner;
use embassy_mspm0::Config;
use embassy_mspm0::gpio::{Input, Level, Output, Pull};
use embassy_mspm0::uart::{self, UartTx};
use panic_halt as _;

#[embassy_executor::main]
async fn main(_spawner: Spawner) -> ! {
    let p = embassy_mspm0::init(Config::default());

    let mut uart = UartTx::new_blocking(p.UC4, p.PA0, uart::Config::default()).unwrap();

    // USER_LED1 (green) is active low.
    let mut led = Output::new(p.PD1, Level::High);
    let mut button = Input::new(p.PB0, Pull::Up);

    uart.blocking_write(b"Press the wake button\r\n").unwrap();

    let mut presses = 0u32;

    loop {
        button.wait_for_falling_edge().await;
        presses += 1;
        led.toggle();

        let mut line = heapless::String::<32>::new();
        let _ = write!(line, "Button pressed ({presses})\r\n");
        uart.blocking_write(line.as_bytes()).unwrap();
    }
}
