//! Placeholder for EtherCAT on the LP-AM13E232: checks the EtherCAT hardware the slave controller
//! (ESC) will use, without the ESC itself, which `embassy-am13` does not drive yet.
//!
//! It resets the two DP83826 Ethernet PHYs (ESC_PHY_RESETn, PB1), reads their identifier and status
//! over MDIO (MDC on PB16, MDIO on PC20, bit-banged with GPIO), and prints the result on the
//! backchannel UART (UC4 on PA0 and PA1, 115200 baud). The PHYs are at MDIO addresses 0 (port 0,
//! U18) and 1 (port 1, U21). The pins are those of TI's CiA402 EtherCAT example for this
//! LaunchPad. The ESC RUN LED (PB2) then blinks, driven as a GPIO.
//!
//! See README.md in this folder for what an EtherCAT port needs.

#![no_std]
#![no_main]

use core::fmt::Write;

use embassy_am13::Config;
use embassy_am13::gpio::{Flex, Level, Output, Pull};
use embassy_am13::uart::{self, UartTx};
use embassy_executor::Spawner;
use embassy_time::Timer;
use panic_halt as _;

/// The identifier of a DP83826 in PHYIDR1 (register 2): TI's OUI bits 3 to 18.
const DP83826_PHYIDR1: u16 = 0x2000;

/// A bit-banged MDIO bus (IEEE 802.3 clause 22).
struct Mdio<'d> {
    mdc: Output<'d>,
    mdio: Flex<'d>,
}

impl Mdio<'_> {
    /// Half an MDC period: about 1 µs at 32 MHz, well below the 2.5 MHz MDC limit.
    fn half_period() {
        cortex_m::asm::delay(32);
    }

    fn write_bit(&mut self, bit: bool) {
        self.mdio.set_level(bit.into());
        Self::half_period();
        self.mdc.set_high();
        Self::half_period();
        self.mdc.set_low();
    }

    /// The PHY changes MDIO after the rising edge of MDC, so the bit is sampled just before it.
    fn read_bit(&mut self) -> bool {
        Self::half_period();
        let bit = self.mdio.is_high();
        self.mdc.set_high();
        Self::half_period();
        self.mdc.set_low();
        bit
    }

    /// Read register `reg` of the PHY at `phy`.
    fn read(&mut self, phy: u8, reg: u8) -> u16 {
        self.mdio.set_as_output();

        // Preamble: 32 ones.
        for _ in 0..32 {
            self.write_bit(true);
        }
        // Start (01), read (10), PHY address, register address.
        let frame = (0b0110u16 << 10) | ((phy as u16 & 0x1F) << 5) | (reg as u16 & 0x1F);
        for i in (0..14).rev() {
            self.write_bit(frame & (1 << i) != 0);
        }

        // Turnaround: the PHY drives the second bit low.
        self.mdio.set_as_input();
        self.read_bit();
        self.read_bit();

        let mut value = 0;
        for _ in 0..16 {
            value = (value << 1) | self.read_bit() as u16;
        }

        // One idle cycle.
        self.read_bit();
        value
    }
}

#[embassy_executor::main]
async fn main(_spawner: Spawner) -> ! {
    let p = embassy_am13::init(Config::default());

    let mut uart = UartTx::new_blocking(p.UC4, p.PA0, uart::Config::default()).unwrap();
    uart.blocking_write(b"EtherCAT placeholder: PHY check\r\n").unwrap();

    // Reset the PHYs. The DP83826 needs a reset pulse of at least 25 us and is ready for MDIO
    // within 50 ms.
    let mut phy_reset = Output::new(p.PB1, Level::Low);
    Timer::after_millis(10).await;
    phy_reset.set_high();
    Timer::after_millis(100).await;

    let mut mdio = Flex::new(p.PC20);
    mdio.set_pull(Pull::Up);
    mdio.set_high();
    let mut mdio = Mdio {
        mdc: Output::new(p.PB16, Level::Low),
        mdio,
    };

    let mut found = 0;
    for phy in 0..2 {
        let id1 = mdio.read(phy, 2);
        let id2 = mdio.read(phy, 3);
        // BMSR (register 1) latches a link loss: read it twice for the current state.
        mdio.read(phy, 1);
        let bmsr = mdio.read(phy, 1);

        let is_dp83826 = id1 == DP83826_PHYIDR1;
        found += is_dp83826 as u32;

        let mut line = heapless::String::<96>::new();
        let _ = write!(
            line,
            "PHY {phy}: PHYIDR1 {id1:#06x} PHYIDR2 {id2:#06x}{}, link {}\r\n",
            if is_dp83826 { " (DP83826)" } else { " (no PHY)" },
            if bmsr & (1 << 2) != 0 { "up" } else { "down" },
        );
        uart.blocking_write(line.as_bytes()).unwrap();
    }

    let mut line = heapless::String::<48>::new();
    let _ = write!(line, "ethercat: {found} of 2 PHYs found\r\n");
    uart.blocking_write(line.as_bytes()).unwrap();

    let mut run_led = Output::new(p.PB2, Level::Low);
    loop {
        run_led.toggle();
        Timer::after_millis(500).await;
    }
}
