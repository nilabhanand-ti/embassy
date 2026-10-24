# EtherCAT (placeholder)

The AM13E232x has an EtherCAT slave controller (ESC, the `ESCSS` peripheral) with two MII ports,
and the LP-AM13E232 wires it to two DP83826 PHYs and RJ45 jacks, with the ESC's EEPROM on PC2
and PC3. `embassy-am13` does not drive the ESC yet.

This folder holds the EtherCAT example. For now `main.rs` only checks the board's EtherCAT
hardware without the ESC: it resets the PHYs (PB1) and reads their identifiers and link status
over MDIO (PB16 and PC20, bit-banged), then blinks the ESC RUN LED (PB2) as a GPIO.

```
cargo run --release --bin ethercat
```

Expected output on the backchannel UART (link up with a cable to an EtherCAT master or switch):

```
EtherCAT placeholder: PHY check
PHY 0: PHYIDR1 0x2000 PHYIDR2 0xa13x (DP83826), link down
PHY 1: PHYIDR1 0x2000 PHYIDR2 0xa13x (DP83826), link down
ethercat: 2 of 2 PHYs found
```

## What an EtherCAT port needs

- A register block for `ESCSS` (the subsystem wrapper: clocks, resets, pin and PHY
  configuration; TI's `dl_escss.c` in the CiA402 example shows the sequence) and access to the
  ESC's process data RAM and registers.
- The ESC pin functions: MII ports 0 and 1, MDIO (PB16/PC20), PHY clock (PC0, 25 MHz), PHY
  reset (PB1), link status (PC8/PC9), LEDs (PB2/PB3/PC18/PC1) and EEPROM I2C (PC2/PC3). The pin
  traits are in the metadata (`ESCSS` peripheral).
- The interrupts `ECAT_INTn`, `ECAT_SYNC0`, `ECAT_SYNC1` and `ECAT_RST`.
- An EtherCAT slave stack on top: the application layer state machine, mailbox (CoE) and
  process data, for example a CiA402 drive profile as in TI's `am13e232_cia402` example.
