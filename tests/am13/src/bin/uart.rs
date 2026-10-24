#![no_std]
#![no_main]

#[cfg(feature = "am13e232n9")]
teleprobe_meta::target!(b"lp-am13e232");

#[cfg(feature = "am13e23019")]
teleprobe_meta::target!(b"lp-am13e230");

use defmt::{assert_eq, unwrap, *};
use defmt_rtt as _;
use embassy_am13::mode::Blocking;
use embassy_am13::uart::{Config, Error, Uart};
use embassy_executor::Spawner;
use panic_probe as _;

fn read<const N: usize>(uart: &mut Uart<'_, Blocking>) -> Result<[u8; N], Error> {
    let mut buf = [255; N];
    uart.blocking_read(&mut buf)?;
    Ok(buf)
}

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_am13::init(Default::default());
    info!("Hello World!");

    // UC4 on PA0 and PA1, the XDS110 backchannel on both LaunchPads. The UART runs in loopback, so
    // nothing is received from the pins.
    let (mut tx, mut rx, mut uart) = (p.PA0, p.PA1, p.UC4);

    // The UNICOMM UARTs run from MCLK/2 (32 MHz).
    const BAUD_RATES: &[u32] = &[1200, 9600, 19200, 57600, 115200, 460800, 921600, 2_000_000];

    for &rate in BAUD_RATES {
        info!("{} baud", rate);

        let mut config = Config::default();
        // The UART has a loopback mode to allow self test.
        config.loop_back_enable = true;
        config.baudrate = rate;

        let mut uart = unwrap!(Uart::new_blocking(
            uart.reborrow(),
            rx.reborrow(),
            tx.reborrow(),
            config
        ));

        // We can't send too many bytes, they have to fit in the FIFO.
        // This is because we aren't sending+receiving at the same time.

        let data = [0xC0, 0xDE];
        unwrap!(uart.blocking_write(&data));
        assert_eq!(unwrap!(read(&mut uart)), data);
    }

    info!("Test OK");
    cortex_m::asm::bkpt();
}
