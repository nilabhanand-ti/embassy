//! PWM on TIMG12: channel 1 on PA27, 1 kHz, sweeping the duty cycle from 0 to 100% and
//! back every two seconds.
//!
//! Put an LED (with a resistor) or a scope on PA27. The duty cycle is printed on the backchannel
//! UART (UC4 on PA0 and PA1, 115200 baud).

#![no_std]
#![no_main]

use core::fmt::Write;

use embassy_am13::Config;
use embassy_am13::gpio::Pull;
use embassy_am13::tim::simple_pwm::{self, PwmPin, SimplePwm};
use embassy_am13::uart::{self, UartTx};
use embassy_executor::Spawner;
use embassy_time::Timer;
use panic_halt as _;

#[embassy_executor::main]
async fn main(_spawner: Spawner) -> ! {
    let p = embassy_am13::init(Config::default());

    let mut uart = UartTx::new_blocking(p.UC4, p.PA0, uart::Config::default()).unwrap();

    let mut config = simple_pwm::Config::default();
    config.frequency = 1_000;
    let mut pwm =
        SimplePwm::new_2ch_32bit(p.TIMG12, None, Some(PwmPin::new(p.PA27, Pull::None, false)), config).unwrap();

    let mut ch1 = pwm.ch1();
    ch1.enable();

    let mut line = heapless::String::<64>::new();
    let _ = write!(
        line,
        "PWM at {} Hz, max duty {}\r\n",
        ch1.get_frequency(),
        ch1.max_duty_cycle()
    );
    uart.blocking_write(line.as_bytes()).unwrap();

    let max = ch1.max_duty_cycle();
    loop {
        for percent in (0..=100).chain((0..100).rev()) {
            ch1.set_duty_cycle(max * percent / 100).unwrap();
            if percent % 25 == 0 {
                let mut line = heapless::String::<32>::new();
                let _ = write!(line, "duty {percent}%\r\n");
                uart.blocking_write(line.as_bytes()).unwrap();
            }
            Timer::after_millis(10).await;
        }
    }
}
