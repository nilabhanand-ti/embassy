#![no_std]
#![no_main]

#[cfg(feature = "am13e232n9")]
teleprobe_meta::target!(b"lp-am13e232");

#[cfg(feature = "am13e23019")]
teleprobe_meta::target!(b"lp-am13e230");

use defmt::{assert_eq, unwrap, *};
use defmt_rtt as _;
use embassy_am13::spi::{self, MODE_0, MODE_3, Spi};
use embassy_am13::{bind_interrupts, peripherals};
use embassy_executor::Spawner;
use panic_probe as _;

#[cfg(feature = "am13e232n9")]
bind_interrupts!(struct Irqs {
    UC0 => spi::InterruptHandler<peripherals::UC0>;
});

#[cfg(feature = "am13e23019")]
bind_interrupts!(struct Irqs {
    UC3 => spi::InterruptHandler<peripherals::UC3>;
});

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_am13::init(Default::default());
    info!("Hello World!");

    // The SPI runs in loopback, so nothing needs to be connected.
    #[cfg(feature = "am13e232n9")]
    let (uc, sclk, pico, poci) = (p.UC0, p.PD7, p.PD8, p.PD6);
    #[cfg(feature = "am13e23019")]
    let (uc, sclk, pico, poci) = (p.UC3, p.PB5, p.PB10, p.PA20);

    let mut config = spi::Config::default();
    let mut spi = unwrap!(Spi::new(uc, sclk, pico, poci, Irqs, config));
    spi.set_loopback(true);

    for frequency in [100_000, 1_000_000, 4_000_000, 16_000_000] {
        for mode in [MODE_0, MODE_3] {
            info!("{} Hz", frequency);
            config.frequency = frequency;
            config.mode = mode;
            unwrap!(spi.set_config(&config));

            let tx: [u8; 32] = core::array::from_fn(|i| (i as u8).wrapping_mul(37) ^ 0xA5);

            let mut rx = [0u8; 32];
            unwrap!(spi.blocking_transfer(&mut rx, &tx));
            assert_eq!(rx, tx);

            let mut rx = [0u8; 32];
            unwrap!(spi.transfer(&mut rx, &tx).await);
            assert_eq!(rx, tx);

            let mut words = tx;
            unwrap!(spi.transfer_in_place(&mut words).await);
            assert_eq!(words, tx);

            let tx16: [u16; 8] = core::array::from_fn(|i| 0x1234u16.wrapping_mul(i as u16 + 1));
            let mut rx16 = [0u16; 8];
            unwrap!(spi.blocking_transfer(&mut rx16, &tx16));
            assert_eq!(rx16, tx16);
        }
    }

    info!("Test OK");
    cortex_m::asm::bkpt();
}
