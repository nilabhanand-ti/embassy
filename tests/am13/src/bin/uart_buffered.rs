#![no_std]
#![no_main]

#[cfg(feature = "am13e232n9")]
teleprobe_meta::target!(b"lp-am13e232");

#[cfg(feature = "am13e23019")]
teleprobe_meta::target!(b"lp-am13e230");

use defmt::{assert_eq, unwrap, *};
use defmt_rtt as _;
use embassy_am13::uart::{BufferedInterruptHandler, BufferedUart, Config};
use embassy_am13::{bind_interrupts, peripherals};
use embassy_executor::Spawner;
use panic_probe as _;

bind_interrupts!(struct Irqs {
    UC4 => BufferedInterruptHandler<peripherals::UC4>;
});

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_am13::init(Default::default());
    info!("Hello World!");

    // UC4 on PA0 and PA1, the XDS110 backchannel on both LaunchPads. The UART runs in loopback, so
    // nothing is received from the pins.
    let (mut tx, mut rx, mut uart) = (p.PA0, p.PA1, p.UC4);

    {
        use embedded_io_async::{Read, Write};

        let mut config = Config::default();
        config.loop_back_enable = true;

        let tx_buf = &mut [0u8; 16];
        let rx_buf = &mut [0u8; 16];
        let mut uart = unwrap!(BufferedUart::new(
            uart.reborrow(),
            tx.reborrow(),
            rx.reborrow(),
            Irqs,
            tx_buf,
            rx_buf,
            config
        ));

        let mut buf = [0; 16];
        for (j, b) in buf.iter_mut().enumerate() {
            *b = j as u8;
        }

        unwrap!(uart.write_all(&buf).await);
        unwrap!(uart.flush().await);

        unwrap!(uart.read_exact(&mut buf).await);
        for (j, b) in buf.iter().enumerate() {
            assert_eq!(*b, j as u8);
        }

        // Buffer is unclogged, should be able to write again.
        unwrap!(uart.write_all(&buf).await);
        unwrap!(uart.flush().await);

        unwrap!(uart.read_exact(&mut buf).await);
        for (j, b) in buf.iter().enumerate() {
            assert_eq!(*b, j as u8);
        }
    }

    info!("Blocking buffered");
    {
        use embedded_io::{Read, Write};

        let mut config = Config::default();
        config.loop_back_enable = true;

        let tx_buf = &mut [0u8; 16];
        let rx_buf = &mut [0u8; 16];
        let mut uart = unwrap!(BufferedUart::new(
            uart.reborrow(),
            tx.reborrow(),
            rx.reborrow(),
            Irqs,
            tx_buf,
            rx_buf,
            config
        ));

        let mut buf = [0; 16];

        for (j, b) in buf.iter_mut().enumerate() {
            *b = j as u8;
        }

        unwrap!(uart.write_all(&buf));
        unwrap!(uart.blocking_flush());
        unwrap!(uart.read_exact(&mut buf));

        for (j, b) in buf.iter().enumerate() {
            assert_eq!(*b, j as u8);
        }

        // Buffer is unclogged, should be able to write again.
        unwrap!(uart.write_all(&buf));
        unwrap!(uart.blocking_flush());
        unwrap!(uart.read_exact(&mut buf));

        for (j, b) in buf.iter().enumerate() {
            assert_eq!(*b, j as u8, "at {}", j);
        }
    }

    info!("Test OK");
    cortex_m::asm::bkpt();
}
