//! Reads the ADC once a second: the temperature sensor, the analog supply (VDDA) through the
//! internal test channel, and the analog input on PA28 (ADC0 channel 11).
//!
//! The readings are printed on the backchannel UART (UC4 on PA0 and PA1, 115200 baud). With the
//! default internal reference the range is 0 V to 3.3 V: 4095 is 3.3 V.

#![no_std]
#![no_main]

use core::fmt::Write;

use embassy_am13::adc::{self, Adc, Channel, TestSignal};
use embassy_am13::uart::{self, UartTx};
use embassy_am13::{Config, bind_interrupts, peripherals};
use embassy_executor::Spawner;
use embassy_time::Timer;
use panic_halt as _;

bind_interrupts!(struct Irqs {
    ADC0_INT1 => adc::InterruptHandler<peripherals::ADC0>;
});

#[embassy_executor::main]
async fn main(_spawner: Spawner) -> ! {
    let p = embassy_am13::init(Config::default());

    let mut uart = UartTx::new_blocking(p.UC4, p.PA0, uart::Config::default()).unwrap();

    let mut adc = Adc::new(p.ADC0, Irqs, adc::Config::default());
    let mut temperature = Channel::temperature_sensor();
    let mut vdda = Channel::internal_test(TestSignal::Vdda);
    let mut pin = Channel::pin(p.PA28);
    let calibration = adc::temperature_calibration();

    loop {
        let t = adc.read(&mut temperature).await;
        let v = adc.read(&mut vdda).await;
        let a = adc.read(&mut pin).await;

        let mut line = heapless::String::<96>::new();
        let _ = write!(line, "temperature {t}");
        if let Some(celsius) = calibration.celsius(t) {
            let _ = write!(line, " ({} C)", celsius as i32);
        }
        // mV = raw * 3300 / 4095
        let _ = write!(
            line,
            ", VDDA {v} ({} mV), PA28 {a} ({} mV)\r\n",
            v as u32 * 3300 / 4095,
            a as u32 * 3300 / 4095
        );
        uart.blocking_write(line.as_bytes()).unwrap();

        Timer::after_secs(1).await;
    }
}
