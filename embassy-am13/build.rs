use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::{env, fs};

use common::CfgSet;
use mspm0_metapac::metadata::{ALL_CHIPS, METADATA};
use proc_macro2::{Ident, Literal, Span, TokenStream};
use quote::{format_ident, quote};

#[path = "./build_common.rs"]
mod common;

fn main() {
    let mut cfgs = common::CfgSet::new();
    common::set_target_cfgs(&mut cfgs);

    generate_code(&mut cfgs);
}

fn generate_code(cfgs: &mut CfgSet) {
    #[cfg(any(feature = "rt"))]
    println!(
        "cargo:rustc-link-search={}",
        PathBuf::from(env::var_os("OUT_DIR").unwrap()).display(),
    );

    cfgs.declare_all(&["gpio_pb", "gpio_pc", "gpio_pd"]);

    let chip_name = match env::vars()
        .map(|(a, _)| a)
        .filter(|x| x.starts_with("CARGO_FEATURE_AM13"))
        .get_one()
    {
        Ok(x) => x,
        Err(GetOneError::None) => panic!("No am13xx Cargo feature enabled"),
        Err(GetOneError::Multiple) => panic!("Multiple am13xx Cargo features enabled"),
    }
    .strip_prefix("CARGO_FEATURE_")
    .unwrap()
    .to_ascii_lowercase()
    .replace('_', "-");

    eprintln!("chip: {chip_name}");

    cfgs.enable_all(&get_chip_cfgs(&chip_name));
    for chip in ALL_CHIPS.iter().filter(|c| c.starts_with("am13")) {
        cfgs.declare_all(&get_chip_cfgs(chip));
    }

    let mut singletons = get_singletons(cfgs);

    time_driver(&mut singletons, cfgs);
    pin_features(&mut singletons);

    let mut g = TokenStream::new();

    g.extend(generate_singletons(&singletons));
    g.extend(generate_pincm_mapping());
    g.extend(generate_pin());
    g.extend(generate_timers());
    g.extend(generate_interrupts());
    g.extend(generate_peripheral_instances());
    g.extend(generate_pin_trait_impls());
    g.extend(generate_dma_channel_count());

    let out_dir = &PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let out_file = out_dir.join("_generated.rs").to_string_lossy().to_string();
    fs::write(&out_file, g.to_string()).unwrap();
    rustfmt(&out_file);
}

/// The family of a chip, for what differs between the AM13E230x and the AM13E232x.
fn get_chip_cfgs(chip_name: &str) -> Vec<String> {
    let mut cfgs = Vec::new();

    if chip_name.starts_with("am13e230") {
        cfgs.push("am13e230x".to_string());
    }

    if chip_name.starts_with("am13e232") {
        cfgs.push("am13e232x".to_string());
    }

    cfgs
}

fn generate_dma_channel_count() -> TokenStream {
    let count = METADATA.dma_channels.len();

    quote! { pub const DMA_CHANNELS: usize = #count; }
}

#[derive(Debug, Clone)]
struct Singleton {
    name: String,

    /// `#[cfg]` guard which enables this singleton instance to be obtained.
    cfg: Option<TokenStream>,
}

fn get_singletons(cfgs: &mut common::CfgSet) -> Vec<Singleton> {
    let mut singletons = Vec::<Singleton>::new();

    for peripheral in METADATA.peripherals {
        // Some peripherals do not generate a singleton, but generate a singleton for each pin.
        let skip_peripheral_singleton = match peripheral.kind {
            "gpio" => {
                // Also enable ports that are present.
                match peripheral.name {
                    "GPIOB" => cfgs.enable("gpio_pb"),
                    "GPIOC" => cfgs.enable("gpio_pc"),
                    "GPIOD" => cfgs.enable("gpio_pd"),
                    _ => (),
                }

                true
            }

            // Each channel gets a singleton, handled separately.
            "dma" => true,

            // Managed by the HAL.
            "iomux" => true,

            // The registers of each mode of a UNICOMM instance, driven through the instance's
            // singleton (UCx).
            "unicommuart" | "unicommspi" | "unicommi2cc" | "unicommi2ct" => true,

            // The results of an ADC, read through the ADC's singleton.
            "adcresult" => true,

            _ => false,
        };

        if !skip_peripheral_singleton {
            singletons.push(Singleton {
                name: peripheral.name.to_string(),
                cfg: None,
            });
        }

        // Generate each GPIO pin singleton
        if peripheral.name.starts_with("GPIO") {
            for pin in peripheral.pins {
                singletons.push(Singleton {
                    name: pin.signal.to_string(),
                    cfg: None,
                });
            }
        }
    }

    // TODO: Generate this more generally for other signals (FCC_IN, HFCLKIN)
    singletons.push(Singleton {
        name: String::from("CLK_OUT"),
        cfg: None,
    });

    // DMA channels get their own singletons
    for dma_channel in METADATA.dma_channels.iter() {
        singletons.push(Singleton {
            name: format!("DMA_CH{}", dma_channel.number),
            cfg: None,
        });
    }

    singletons.sort_by(|a, b| a.name.cmp(&b.name));
    singletons
}

fn generate_pincm_mapping() -> TokenStream {
    let pincms = METADATA.pins.iter().map(|mapping| {
        let port_letter = mapping.pin.strip_prefix("P").unwrap();
        let port_base = (port_letter.chars().next().unwrap() as u8 - b'A') * 32;
        let pin_number = mapping.pin[2..].parse::<u8>().unwrap();

        let num = port_base + pin_number;

        // The metadata numbers PINCM from 1.
        let pincm = Literal::u8_unsuffixed(mapping.pincm - 1);
        quote! {
            #num => #pincm
        }
    });

    quote! {
        #[doc = "Get the mapping from GPIO pin port to IOMUX PINCM index."]
        pub(crate) fn gpio_pincm(pin_port: u8) -> u8 {
            match pin_port {
                #(#pincms),*,
                _ => unreachable!(),
            }
        }
    }
}

fn generate_pin() -> TokenStream {
    let pin_impls = METADATA.pins.iter().map(|pin| {
        let name = Ident::new(&pin.pin, Span::call_site());
        let port_letter = pin.pin.strip_prefix("P").unwrap();
        let port_letter = port_letter.chars().next().unwrap();
        let pin_number = Literal::u8_unsuffixed(pin.pin[2..].parse::<u8>().unwrap());

        let port = Ident::new(&format!("Port{}", port_letter), Span::call_site());

        quote! {
            impl_pin!(#name, crate::gpio::Port::#port, #pin_number);
        }
    });

    quote! {
        #(#pin_impls)*
    }
}

/// The timers the time driver can run on: TIMG4, a 16-bit timer with a prescaler. TIMG12 is a
/// 32-bit timer without one, which the time driver does not support.
const TIME_DRIVER_TIMERS: &[&str] = &["TIMG4"];

fn time_driver(singletons: &mut [Singleton], cfgs: &mut CfgSet) {
    for timer in TIME_DRIVER_TIMERS {
        cfgs.declare(format!("time_driver_{}", timer.to_lowercase()));
    }

    let time_driver = match env::vars()
        .map(|(a, _)| a)
        .filter(|x| x.starts_with("CARGO_FEATURE_TIME_DRIVER_"))
        .get_one()
    {
        Ok(x) => Some(
            x.strip_prefix("CARGO_FEATURE_TIME_DRIVER_")
                .unwrap()
                .to_ascii_lowercase(),
        ),
        Err(GetOneError::None) => None,
        Err(GetOneError::Multiple) => panic!("Multiple time-driver-xxx Cargo features enabled"),
    };

    let selected_timer = match time_driver.as_deref() {
        None => return,
        Some("any") => TIME_DRIVER_TIMERS[0],
        Some(t) => TIME_DRIVER_TIMERS
            .iter()
            .find(|tim| tim.eq_ignore_ascii_case(t))
            .unwrap_or_else(|| panic!("unknown time_driver {t:?}")),
    };

    cfgs.enable(format!("time_driver_{}", selected_timer.to_lowercase()));

    // The timer of the time driver is not available to the application.
    let feature = format!("time-driver-{}", selected_timer.to_lowercase());
    for singleton in singletons.iter_mut().filter(|s| s.name == selected_timer) {
        singleton.cfg = Some(quote! { #[cfg(not(any(feature = "time-driver-any", feature = #feature)))] });
    }
}

fn pin_features(singletons: &mut [Singleton]) {
    let debugss = METADATA
        .peripherals
        .iter()
        .find(|p| p.name == "DEBUGSS")
        .expect("Could not find DEBUGSS peripheral");

    for pin in debugss.pins.iter() {
        let pin = singletons
            .iter_mut()
            .find(|s| s.name == pin.pin)
            .expect("Could not find SWD pin to cfg gate");

        pin.cfg = Some(quote! { #[cfg(feature = "swd-pins-as-gpio")] });
    }
}

fn generate_singletons(singletons: &[Singleton]) -> TokenStream {
    let singletons_peripherals_struct = singletons
        .iter()
        .map(|s| {
            let cfg = s.cfg.clone().unwrap_or_default();
            let ident = format_ident!("{}", s.name);

            quote! {
                #cfg
                #ident
            }
        })
        .collect::<Vec<_>>();

    let singletons_peripherals_def = singletons
        .iter()
        .map(|s| format_ident!("{}", s.name))
        .collect::<Vec<_>>();

    quote! {
        embassy_hal_internal::peripherals_definition!(#(#singletons_peripherals_def),*);
        embassy_hal_internal::peripherals_struct!(#(#singletons_peripherals_struct),*);
    }
}

fn generate_timers() -> TokenStream {
    let timer_impls = METADATA
        .peripherals
        .iter()
        .filter(|p| p.kind == "tim")
        .flat_map(|peripheral| {
            let name = Ident::new(peripheral.name, Span::call_site());

            // TIMG4 is a 16-bit timer with an 8-bit prescaler, TIMG12 a 32-bit timer without
            // one. Both have two capture/compare channels.
            match peripheral.name {
                "TIMG4" => vec![
                    quote! { impl_tim_instance!(#name, prescaler: true, width: Bits16, channels: 2); },
                    quote! { impl_tim_instance_general_2ch!(#name); },
                ],
                "TIMG12" => vec![
                    quote! { impl_tim_instance!(#name, prescaler: false, width: Bits32, channels: 2); },
                    quote! { impl_tim_instance_general_32bit!(#name); },
                    quote! { impl_tim_instance_general_2ch!(#name); },
                ],
                other => panic!("unknown timer {other}"),
            }
        });

    quote! {
        #(#timer_impls)*
    }
}

fn generate_interrupts() -> TokenStream {
    let interrupts: Vec<Ident> = METADATA
        .interrupts
        .iter()
        .map(|interrupt| Ident::new(interrupt.name, Span::call_site()))
        .collect();

    quote! {
        embassy_hal_internal::interrupt_mod! {
            #(#interrupts),*
        }
    }
}

/// The UNICOMM instances whose I2C controller runs bursts (CTR.BLEN), the only ones the I2C driver
/// supports (dl_unicomm.h, `DL_UNICOMM_isAdvanceFeaturesEnabled`).
const I2C_INSTANCES: &[&str] = &["UC2", "UC5"];

/// The register block of a mode of a UNICOMM instance (`UC0_SPI` for `UC0` and `SPI`), if the
/// instance has that mode and the HAL supports it.
fn unicomm_mode(instance: &str, mode: &str) -> Option<&'static str> {
    if mode == "I2CC" && !I2C_INSTANCES.contains(&instance) {
        return None;
    }

    let name = format!("{instance}_{mode}");

    METADATA
        .peripherals
        .iter()
        .find(|p| p.name == name && p.version.is_some())
        .map(|p| p.name)
}

fn generate_peripheral_instances() -> TokenStream {
    let mut impls = Vec::<TokenStream>::new();

    for peripheral in METADATA.peripherals {
        let peri = format_ident!("{}", peripheral.name);

        match peripheral.kind {
            "unicomm" => {
                if let Some(uart) = unicomm_mode(peripheral.name, "UART") {
                    let uart = format_ident!("{}", uart);
                    impls.push(quote! { impl_uart_instance!(#peri, #uart); });
                }
                if let Some(spi) = unicomm_mode(peripheral.name, "SPI") {
                    let spi = format_ident!("{}", spi);
                    impls.push(quote! { impl_spi_instance!(#peri, #spi); });
                }
                if let Some(i2c) = unicomm_mode(peripheral.name, "I2CC") {
                    let i2c = format_ident!("{}", i2c);
                    impls.push(quote! { impl_i2c_instance!(#peri, #i2c); });
                }
            }
            "adc" => {
                let result = format_ident!("{}RESULT", peripheral.name);
                let irq = format_ident!("{}_INT1", peripheral.name);
                impls.push(quote! { impl_adc_instance!(#peri, #result, #irq); });
            }
            "wwdt" => impls.push(quote! { impl_wwdt_instance!(#peri); }),
            _ => (),
        }
    }

    for dma_channel in METADATA.dma_channels.iter() {
        let peri = format_ident!("DMA_CH{}", dma_channel.number);
        let num = dma_channel.number;

        if dma_channel.full {
            impls.push(quote! { impl_full_dma_channel!(#peri, #num); });
        } else {
            impls.push(quote! { impl_dma_channel!(#peri, #num); });
        }
    }

    quote! {
        #(#impls)*
    }
}

fn generate_pin_trait_impls() -> TokenStream {
    let mut impls = Vec::<TokenStream>::new();

    for peripheral in METADATA.peripherals {
        let uart = unicomm_mode(peripheral.name, "UART").is_some();
        let spi = unicomm_mode(peripheral.name, "SPI").is_some();
        let i2c = unicomm_mode(peripheral.name, "I2CC").is_some();

        for pin in peripheral.pins {
            let pin_name = format_ident!("{}", pin.pin);
            let peri = format_ident!("{}", peripheral.name);
            let pf = pin.pf;

            let tokens = match (peripheral.kind, pin.signal) {
                ("adc", s) => {
                    let channel = s.parse::<u8>().unwrap();
                    Some(quote! { impl_adc_pin!(#peri, #pin_name, #channel); })
                }
                ("sysctl", "CLK_OUT") => Some(quote! { impl_clk_out_pin!(#pin_name, #pf); }),
                ("tim", "CCP0") => Some(quote! { impl_tim_pin!(#peri, #pin_name, #pf, Ch0); }),
                ("tim", "CCP1") => Some(quote! { impl_tim_pin!(#peri, #pin_name, #pf, Ch1); }),

                ("unicomm", "TX") if uart => Some(quote! { impl_uart_tx_pin!(#peri, #pin_name, #pf); }),
                ("unicomm", "RX") if uart => Some(quote! { impl_uart_rx_pin!(#peri, #pin_name, #pf); }),
                ("unicomm", "CTS") if uart => Some(quote! { impl_uart_cts_pin!(#peri, #pin_name, #pf); }),
                ("unicomm", "RTS") if uart => Some(quote! { impl_uart_rts_pin!(#peri, #pin_name, #pf); }),

                ("unicomm", "SCLK") if spi => Some(quote! { impl_spi_sclk_pin!(#peri, #pin_name, #pf); }),
                ("unicomm", "PICO") if spi => Some(quote! { impl_spi_pico_pin!(#peri, #pin_name, #pf); }),
                ("unicomm", "POCI") if spi => Some(quote! { impl_spi_poci_pin!(#peri, #pin_name, #pf); }),

                ("unicomm", "SCL") if i2c => Some(quote! { impl_i2c_scl_pin!(#peri, #pin_name, #pf); }),
                ("unicomm", "SDA") if i2c => Some(quote! { impl_i2c_sda_pin!(#peri, #pin_name, #pf); }),

                _ => None,
            };

            if let Some(tokens) = tokens {
                impls.push(tokens);
            }
        }
    }

    quote! {
        #(#impls)*
    }
}

/// rustfmt a given path.
/// Failures are logged to stderr and ignored.
fn rustfmt(path: impl AsRef<Path>) {
    let path = path.as_ref();
    match Command::new("rustfmt").args([path]).output() {
        Err(e) => {
            eprintln!("failed to exec rustfmt {:?}: {:?}", path, e);
        }
        Ok(out) => {
            if !out.status.success() {
                eprintln!("rustfmt {:?} failed:", path);
                eprintln!("=== STDOUT:");
                std::io::stderr().write_all(&out.stdout).unwrap();
                eprintln!("=== STDERR:");
                std::io::stderr().write_all(&out.stderr).unwrap();
            }
        }
    }
}

enum GetOneError {
    None,
    Multiple,
}

trait IteratorExt: Iterator {
    fn get_one(self) -> Result<Self::Item, GetOneError>;
}

impl<T: Iterator> IteratorExt for T {
    fn get_one(mut self) -> Result<Self::Item, GetOneError> {
        match self.next() {
            None => Err(GetOneError::None),
            Some(res) => match self.next() {
                Some(_) => Err(GetOneError::Multiple),
                None => Ok(res),
            },
        }
    }
}
