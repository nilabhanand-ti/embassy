#![no_std]
#![no_main]

#[cfg(feature = "am13e232n9")]
teleprobe_meta::target!(b"lp-am13e232");

#[cfg(feature = "am13e23019")]
teleprobe_meta::target!(b"lp-am13e230");

use defmt::{assert, *};
use defmt_rtt as _;
use embassy_am13::adc::{self, Adc, Channel, TestSignal, Vref};
use embassy_am13::{bind_interrupts, peripherals};
use embassy_executor::Spawner;
use panic_probe as _;

bind_interrupts!(struct Irqs {
    ADC0_INT1 => adc::InterruptHandler<peripherals::ADC0>;
});

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let mut p = embassy_am13::init(Default::default());
    info!("Hello World!");

    let vddcore = {
        let mut adc = Adc::new(p.ADC0.reborrow(), Irqs, adc::Config::default());

        let vssa = adc.blocking_read(&mut Channel::internal_test(TestSignal::Vssa));
        let vreflo = adc.read(&mut Channel::vreflo()).await;
        let vdda = adc.read(&mut Channel::internal_test(TestSignal::Vdda)).await;
        let vddcore = adc.read(&mut Channel::internal_test(TestSignal::VddCore)).await;
        let temperature = adc.read(&mut Channel::temperature_sensor()).await;
        info!(
            "VSSA {} VREFLO {} VDDA {} VDDCORE {} temperature {}",
            vssa, vreflo, vdda, vddcore, temperature
        );

        assert!(vssa < 100);
        assert!(vreflo < 100);
        // With the internal 3.3 V reference, VDDA is full scale.
        assert!(vdda > 3900);
        // VDDCORE is about 1.2 V.
        assert!((1000..2500).contains(&vddcore));
        if let Some(celsius) = adc::temperature_calibration().celsius(temperature) {
            info!("temperature {} C", celsius);
            assert!((0.0..70.0).contains(&celsius));
        }

        vddcore
    };

    // With the 2.5 V reference, the same voltage reads higher.
    let mut config = adc::Config::default();
    config.vref = Vref::Internal2v5;
    let mut adc = Adc::new_blocking(p.ADC0.reborrow(), config);
    let vddcore_2v5 = adc.blocking_read(&mut Channel::internal_test(TestSignal::VddCore));
    info!("VDDCORE with the 2.5 V reference {}", vddcore_2v5);
    assert!(vddcore_2v5 > vddcore);

    info!("Test OK");
    cortex_m::asm::bkpt();
}
