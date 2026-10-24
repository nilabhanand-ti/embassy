#![no_std]
#![no_main]

#[cfg(feature = "am13e232n9")]
teleprobe_meta::target!(b"lp-am13e232");

#[cfg(feature = "am13e23019")]
teleprobe_meta::target!(b"lp-am13e230");

use defmt::{assert_eq, unwrap, *};
use defmt_rtt as _;
use embassy_am13::gpio::Pull;
use embassy_am13::i2c::{self, Error, I2c};
use embassy_am13::{bind_interrupts, peripherals};
use embassy_executor::Spawner;
use panic_probe as _;

bind_interrupts!(struct Irqs {
    UC2 => i2c::InterruptHandler<peripherals::UC2>;
});

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_am13::init(Default::default());
    info!("Hello World!");

    // UC2 on PA23 (SCL) and PA22 (SDA), with no target on the bus: every address is not
    // acknowledged.
    let mut config = i2c::Config::default();
    config.sda_pull = Pull::Up;
    config.scl_pull = Pull::Up;
    let mut bus = unwrap!(I2c::new(p.UC2, p.PA23, p.PA22, Irqs, config));

    for frequency in [100_000, 400_000, 1_000_000] {
        info!("{} Hz", frequency);
        config.frequency = frequency;
        unwrap!(bus.set_config(&config));

        assert_eq!(bus.bus_levels(), (true, true));

        let mut buf = [0u8; 4];
        assert_eq!(bus.blocking_write(0x2A, &[1, 2, 3]), Err(Error::AddressNack));
        assert_eq!(bus.blocking_read(0x2A, &mut buf), Err(Error::AddressNack));
        assert_eq!(bus.write(0x51, &[0]).await, Err(Error::AddressNack));
        assert_eq!(bus.read(0x51, &mut buf).await, Err(Error::AddressNack));
        assert_eq!(bus.write_read(0x51, &[0], &mut buf).await, Err(Error::AddressNack));

        // The controller releases the bus after each failed transfer.
        assert_eq!(bus.bus_levels(), (true, true));
    }

    assert_eq!(bus.blocking_write(0x2A, &[]), Err(Error::ZeroLengthTransfer));

    info!("Test OK");
    cortex_m::asm::bkpt();
}
