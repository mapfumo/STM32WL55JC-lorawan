//! STM32WL55 Sensor Node with BME688 and OLED Display
//! Sensor-only mode - no LoRaWAN
#![no_std]
#![no_main]

use defmt::*;
use embassy_executor::Spawner;
use embassy_stm32::gpio::{Level, Output, Speed};
use embassy_stm32::i2c::{Config as I2cConfig, ErrorInterruptHandler, EventInterruptHandler, I2c};
use embassy_stm32::peripherals::{I2C2, PA11, PA12};
use embassy_stm32::rcc::*;
use embassy_stm32::time::Hertz;
use embassy_stm32::{bind_interrupts, peripherals};
use embassy_time::Timer;
use embedded_graphics::{
    mono_font::{ascii::FONT_6X10, MonoTextStyle},
    pixelcolor::BinaryColor,
    prelude::*,
    text::Text,
};
use sh1106::{prelude::*, Builder};
use {defmt_rtt as _, panic_probe as _};

// BME688 I2C addresses and registers
const BME688_ADDR_PRIMARY: u8 = 0x76;
const BME688_ADDR_SECONDARY: u8 = 0x77;
const BME688_REG_CHIP_ID: u8 = 0xD0;
const BME688_REG_RESET: u8 = 0xE0;
const BME688_REG_CTRL_HUM: u8 = 0x72;
const BME688_REG_CTRL_MEAS: u8 = 0x74;
const BME688_REG_CONFIG: u8 = 0x75;
const BME688_REG_DATA: u8 = 0x1F;
const BME688_REG_CTRL_GAS_1: u8 = 0x71;
const BME688_REG_RES_HEAT_0: u8 = 0x5A;
const BME688_REG_IDAC_HEAT_0: u8 = 0x50;
const BME688_REG_GAS_WAIT_0: u8 = 0x64;
const BME688_CHIP_ID: u8 = 0x61;
const BME688_RESET_CMD: u8 = 0xB6;

bind_interrupts!(struct I2c2Irqs {
    I2C2_EV => EventInterruptHandler<peripherals::I2C2>;
    I2C2_ER => ErrorInterruptHandler<peripherals::I2C2>;
});

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    info!("====================================");
    info!("  STM32WL55 - Sensor Node");
    info!("  BME688 + OLED Display");
    info!("====================================");

    // Clock configuration
    let mut config = embassy_stm32::Config::default();
    {
        config.rcc.hse = Some(Hse {
            freq: Hertz(32_000_000),
            mode: HseMode::Bypass,
            prescaler: HsePrescaler::DIV1,
        });
        config.rcc.sys = Sysclk::PLL1_R;
        config.rcc.pll = Some(Pll {
            source: PllSource::HSE,
            prediv: PllPreDiv::DIV2,
            mul: PllMul::MUL6,
            divp: None,
            divq: Some(PllQDiv::DIV2),
            divr: Some(PllRDiv::DIV2),
        });
    }
    let p = embassy_stm32::init(config);

    info!("STM32WL55 initialized");

    // Initialize LED
    let mut led = Output::new(p.PB15, Level::Low, Speed::Low);

    // Initialize I2C config
    let mut i2c_config = I2cConfig::default();
    i2c_config.sda_pullup = true;
    i2c_config.scl_pullup = true;

    // Scan I2C bus to find BME688
    info!("Scanning I2C2 bus...");
    let mut bme688_addr = BME688_ADDR_PRIMARY;
    {
        let mut i2c = unsafe {
            I2c::new_blocking(
                I2C2::steal(),
                PA12::steal(),
                PA11::steal(),
                Hertz(100_000),
                i2c_config.clone(),
            )
        };
        
        let mut chip_id_buf = [0u8; 1];
        for addr in [BME688_ADDR_PRIMARY, BME688_ADDR_SECONDARY] {
            if i2c.blocking_write_read(addr, &[BME688_REG_CHIP_ID], &mut chip_id_buf).is_ok() {
                if chip_id_buf[0] == BME688_CHIP_ID {
                    bme688_addr = addr;
                    info!("✓ BME688 found at 0x{:02X}", addr);
                    break;
                }
            }
        }
    }

    // Initialize BME688 sensor
    info!("Initializing BME688...");
    {
        let mut i2c = unsafe {
            I2c::new_blocking(
                I2C2::steal(),
                PA12::steal(),
                PA11::steal(),
                Hertz(100_000),
                i2c_config.clone(),
            )
        };

        // Soft reset
        let _ = i2c.blocking_write(bme688_addr, &[BME688_REG_RESET, BME688_RESET_CMD]);
        Timer::after_millis(10).await;

        // Configure humidity oversampling
        let _ = i2c.blocking_write(bme688_addr, &[BME688_REG_CTRL_HUM, 0x01]);

        // Configure gas sensor
        let _ = i2c.blocking_write(bme688_addr, &[BME688_REG_CTRL_GAS_1, 0x20]);
        let _ = i2c.blocking_write(bme688_addr, &[BME688_REG_RES_HEAT_0, 20]);
        let _ = i2c.blocking_write(bme688_addr, &[BME688_REG_IDAC_HEAT_0, 10]);
        let gas_wait: u8 = (1 << 5) | 25;
        let _ = i2c.blocking_write(bme688_addr, &[BME688_REG_GAS_WAIT_0, gas_wait]);
        let _ = i2c.blocking_write(bme688_addr, &[BME688_REG_CONFIG, 0x00]);

        info!("✓ BME688 initialized");
        Timer::after_millis(1000).await; // Stabilization delay
    }

    // Text style for display
    let text_style = MonoTextStyle::new(&FONT_6X10, BinaryColor::On);

    // Initialize display once at startup to verify it works
    info!("Initializing OLED display at startup...");
    {
        let mut i2c = unsafe {
            I2c::new_blocking(
                I2C2::steal(),
                PA12::steal(),
                PA11::steal(),
                Hertz(100_000),
                i2c_config.clone(),
            )
        };
        let mut display: GraphicsMode<_> = Builder::new()
            .with_size(DisplaySize::Display128x64)
            .connect_i2c(i2c)
            .into();
        
        if display.init().is_ok() {
            Timer::after_millis(50).await;
            display.clear();
            Timer::after_millis(20).await;
            let _ = Text::new("Node1", Point::new(2, 8), text_style).draw(&mut display);
            let _ = Text::new("Starting...", Point::new(2, 20), text_style).draw(&mut display);
            if display.flush().is_ok() {
                info!("✓ OLED display initialized and working");
            } else {
                warn!("✗ OLED display flush failed");
            }
        } else {
            warn!("✗ Failed to initialize OLED display at startup");
        }
    }

    // Sensor state
    let mut temp_int = 0i16;
    let mut hum_int = 0i16;
    let mut press_int = 0u32;
    let mut gas_resistance = 0u32;

    info!("Starting sensor loop...");

    loop {
        led.toggle();

        // Create I2C instance once per loop iteration (like reference code)
        // This ensures clean I2C state each time
        let mut i2c = unsafe {
            I2c::new_blocking(
                I2C2::steal(),
                PA12::steal(),
                PA11::steal(),
                Hertz(100_000),
                i2c_config.clone(),
            )
        };

        // Verify sensor is present by reading chip ID
        let mut chip_id_buf = [0u8; 1];
        let mut sensor_ok = false;
        for retry in 0..3 {
            if i2c.blocking_write_read(bme688_addr, &[BME688_REG_CHIP_ID], &mut chip_id_buf).is_ok() {
                if chip_id_buf[0] == BME688_CHIP_ID {
                    sensor_ok = true;
                    break;
                }
            }
            if retry < 2 {
                Timer::after_millis(50).await;
            }
        }

        if !sensor_ok {
            warn!("✗ BME688 not responding (chip ID: 0x{:02X})", chip_id_buf[0]);
            // Update display with error
            let mut display: GraphicsMode<_> = Builder::new()
                .with_size(DisplaySize::Display128x64)
                .connect_i2c(i2c)
                .into();
            match display.init() {
                Ok(_) => {
                    Timer::after_millis(50).await;
                    display.clear();
                    Timer::after_millis(20).await;
                    if Text::new("Node1", Point::new(2, 8), text_style).draw(&mut display).is_err() {
                        warn!("Failed to draw error title");
                    }
                    if Text::new("Sensor Error", Point::new(2, 20), text_style).draw(&mut display).is_err() {
                        warn!("Failed to draw error message");
                    }
                    if display.flush().is_err() {
                        warn!("Failed to flush error display");
                    }
                }
                Err(_) => {
                    warn!("Failed to init display for error");
                }
            }
            Timer::after_secs(2).await;
            continue;
        }

        // Read current CTRL_MEAS
        let mut ctrl_meas = [0u8; 1];
        if i2c.blocking_write_read(bme688_addr, &[BME688_REG_CTRL_MEAS], &mut ctrl_meas).is_err() {
            warn!("✗ Failed to read CTRL_MEAS");
            drop(i2c);
            Timer::after_secs(2).await;
            continue;
        }

        // Set sleep mode first
        let sleep_mode = (ctrl_meas[0] & 0xFC) | 0x00;
        if i2c.blocking_write(bme688_addr, &[BME688_REG_CTRL_MEAS, sleep_mode]).is_err() {
            warn!("✗ Failed to set sleep mode");
            drop(i2c);
            Timer::after_secs(2).await;
            continue;
        }

        Timer::after_millis(20).await;

        // Trigger forced measurement: 0x25 = temp x1, press x1, forced mode
        if i2c.blocking_write(bme688_addr, &[BME688_REG_CTRL_MEAS, 0x25]).is_err() {
            warn!("✗ Failed to trigger measurement");
            drop(i2c);
            Timer::after_secs(2).await;
            continue;
        }

        // Wait for measurement (200ms like reference)
        Timer::after_millis(200).await;

        // Read data register
        let mut data = [0u8; 10];
        if i2c.blocking_write_read(bme688_addr, &[BME688_REG_DATA], &mut data).is_ok() {
            defmt::debug!(
                "Raw bytes: [{:02X} {:02X} {:02X} {:02X} {:02X} {:02X} {:02X} {:02X} {:02X} {:02X}]",
                data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7], data[8], data[9]
            );

            let press_raw = ((data[0] as u32) << 8) | (data[1] as u32);
            let temp_raw = ((data[3] as u32) << 8) | (data[4] as u32);
            let hum_raw = ((data[6] as u32) << 8) | (data[7] as u32);
            let gas_r_msb = data[8] as u32;
            let gas_r_lsb = data[9] as u32;
            let gas_raw = (gas_r_msb << 2) | (gas_r_lsb >> 6);

            defmt::debug!("Raw: T={} H={} P={} G={}", temp_raw, hum_raw, press_raw, gas_raw);

            // Validation
            let temp_valid = temp_raw > 10000 && temp_raw < 60000 && temp_raw != 32768 && temp_raw != 0 && temp_raw != 65535;
            let hum_valid = hum_raw > 0 && hum_raw < 65535 && hum_raw != 32768 && hum_raw != 65535;
            let press_valid = press_raw > 3000 && press_raw < 50000 && press_raw != 32768 && press_raw != 0 && press_raw != 65535;

            if temp_valid && press_valid {
                if gas_raw == 0x3FF {
                    gas_resistance = 0;
                } else {
                    gas_resistance = gas_raw;
                }

                temp_int = (temp_raw as i32 / 1290) as i16;
                if hum_valid {
                    hum_int = (hum_raw as i32 / 285) as i16;
                    if hum_int > 100 { hum_int = 100; }
                    if hum_int < 0 { hum_int = 0; }
                }
                press_int = ((press_raw * 9) / 2) as u32;

                info!("✓ BME688: {}°C, {}% RH, {} Pa, Gas: {}", temp_int, hum_int, press_int, gas_resistance);

                // Update display using the same I2C instance
                let mut display: GraphicsMode<_> = Builder::new()
                    .with_size(DisplaySize::Display128x64)
                    .connect_i2c(i2c)
                    .into();

                match display.init() {
                    Ok(_) => {
                        Timer::after_millis(50).await;
                        display.clear();
                        Timer::after_millis(20).await;

                        // Draw title
                        if Text::new("Node1", Point::new(2, 8), text_style).draw(&mut display).is_err() {
                            warn!("Failed to draw title");
                        }

                        // Draw sensor data
                        let mut temp_hum_buf = heapless::String::<32>::new();
                        if core::fmt::write(&mut temp_hum_buf, format_args!("T:{}C  H:{}%", temp_int, hum_int)).is_ok() {
                            if Text::new(&temp_hum_buf, Point::new(2, 20), text_style).draw(&mut display).is_err() {
                                warn!("Failed to draw temp/hum");
                            }
                        }

                        let press_hpa = press_int / 100;
                        let mut press_buf = heapless::String::<32>::new();
                        if core::fmt::write(&mut press_buf, format_args!("P:{} hPa", press_hpa)).is_ok() {
                            if Text::new(&press_buf, Point::new(2, 32), text_style).draw(&mut display).is_err() {
                                warn!("Failed to draw pressure");
                            }
                        }

                        let mut gas_buf = heapless::String::<32>::new();
                        if gas_resistance > 0 && gas_resistance != 0x3FF {
                            let _ = core::fmt::write(&mut gas_buf, format_args!("G:{}", gas_resistance));
                        } else {
                            let _ = core::fmt::write(&mut gas_buf, format_args!("G:--"));
                        }
                        if Text::new(&gas_buf, Point::new(2, 44), text_style).draw(&mut display).is_err() {
                            warn!("Failed to draw gas");
                        }

                        Timer::after_millis(20).await;
                        match display.flush() {
                            Ok(_) => {
                                defmt::debug!("Display updated successfully");
                            }
                            Err(_) => {
                                warn!("Display flush failed");
                            }
                        }
                    }
                    Err(_) => {
                        warn!("Failed to init display");
                    }
                }
            } else {
                warn!("✗ Invalid sensor data (T={} H={} P={})", temp_valid, hum_valid, press_valid);
            }
        } else {
            warn!("✗ Failed to read sensor data");
        }

        Timer::after_secs(2).await;
    }
}
