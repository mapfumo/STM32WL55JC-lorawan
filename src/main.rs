//! STM32WL55 LoRaWAN Sensor Node with BME688 and OLED Display
//! Based on working solution architecture with lora-phy
#![no_std]
#![no_main]

mod iv;

use defmt::{info, warn};
use embassy_executor::Spawner;
use embassy_stm32::gpio::{Level, Output, Pin, Speed};
use embassy_stm32::rng::{self, Rng};
use embassy_stm32::spi::Spi;
use embassy_stm32::{bind_interrupts, peripherals};
use embassy_stm32::rcc::*;
use embassy_stm32::time::Hertz;
use embassy_stm32::i2c::{Config as I2cConfig, EventInterruptHandler, ErrorInterruptHandler, I2c};
use embassy_stm32::peripherals::{I2C2, PA11, PA12};
use embassy_time::{Delay, Timer};
use embedded_graphics::{
    mono_font::{ascii::FONT_6X10, MonoTextStyle},
    pixelcolor::BinaryColor,
    prelude::*,
    text::Text,
};
use lora_phy::lorawan_radio::LorawanRadio;
use lora_phy::sx126x::{self, Stm32wl, Sx126x, TcxoCtrlVoltage};
use lora_phy::LoRa;
use lorawan_device::async_device::{region, Device, EmbassyTimer, JoinMode, JoinResponse, SendResponse};
use lorawan_device::default_crypto::DefaultFactory;
use lorawan_device::region::{AU915, Subband};
use lorawan_device::{AppEui, AppKey, DevEui};
use sh1106::{prelude::*, Builder};
use {defmt_rtt as _, panic_probe as _};

use self::iv::{InterruptHandler, Stm32wlInterfaceVariant, SubghzSpiDevice};

// AU915 region configuration
const MAX_TX_POWER: u8 = 14;

// LoRaWAN credentials
const DEV_EUI: [u8; 8] = [0xAC, 0x1F, 0x09, 0xFF, 0xFE, 0x1B, 0xCE, 0x23];
const APP_EUI: [u8; 8] = [0xB1, 0x30, 0xA8, 0x64, 0xC5, 0x29, 0x53, 0x56];
const APP_KEY: [u8; 16] = [
    0xB7, 0x26, 0x73, 0x9B, 0x78, 0xEC, 0x4B, 0x9E,
    0x92, 0x34, 0xE5, 0xD3, 0x5E, 0xA9, 0x68, 0x1B,
];

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

bind_interrupts!(struct Irqs{
    SUBGHZ_RADIO => InterruptHandler;
    RNG => rng::InterruptHandler<peripherals::RNG>;
});

bind_interrupts!(struct I2c2Irqs {
    I2C2_EV => EventInterruptHandler<peripherals::I2C2>;
    I2C2_ER => ErrorInterruptHandler<peripherals::I2C2>;
});

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    info!("====================================");
    info!("  STM32WL55 - LoRaWAN Sensor Node");
    info!("  BME688 + OLED + LoRaWAN");
    info!("====================================");

    // Clock configuration matching working solution
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

    // Initialize LED
    let mut led = Output::new(p.PB15, Level::Low, Speed::Low);

    // Initialize I2C for sensor and display
    let mut i2c_config = I2cConfig::default();
    i2c_config.sda_pullup = true;
    i2c_config.scl_pullup = true;

    // Detect BME688 address
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
        if i2c.blocking_write_read(BME688_ADDR_PRIMARY, &[BME688_REG_CHIP_ID], &mut chip_id_buf).is_ok() {
            if chip_id_buf[0] == BME688_CHIP_ID {
                bme688_addr = BME688_ADDR_PRIMARY;
                info!("✓ BME688 found at 0x{:02X}", bme688_addr);
            }
        } else if i2c.blocking_write_read(BME688_ADDR_SECONDARY, &[BME688_REG_CHIP_ID], &mut chip_id_buf).is_ok() {
            if chip_id_buf[0] == BME688_CHIP_ID {
                bme688_addr = BME688_ADDR_SECONDARY;
                info!("✓ BME688 found at 0x{:02X}", bme688_addr);
            }
        } else {
            warn!("✗ BME688 not found, using default address 0x{:02X}", bme688_addr);
        }
    }

    // Initialize BME688
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
        Timer::after_millis(500).await; // Stabilization delay
    }

    // Initialize LoRa radio
    let ctrl1 = Output::new(p.PC4.degrade(), Level::Low, Speed::High);
    let ctrl2 = Output::new(p.PC5.degrade(), Level::Low, Speed::High);
    let ctrl3 = Output::new(p.PC3.degrade(), Level::High, Speed::High);

    let spi = Spi::new_subghz(p.SUBGHZSPI, p.DMA1_CH1, p.DMA1_CH2);
    let spi = SubghzSpiDevice(spi);
    let use_high_power_pa = true;
    let config = sx126x::Config {
        chip: Stm32wl { use_high_power_pa },
        tcxo_ctrl: Some(TcxoCtrlVoltage::Ctrl1V7),
        use_dcdc: true,
        rx_boost: false,
    };
    let iv = Stm32wlInterfaceVariant::new(Irqs, use_high_power_pa, Some(ctrl1), Some(ctrl2), Some(ctrl3)).unwrap();
    let lora = LoRa::new(Sx126x::new(spi, iv, config), true, Delay).await.unwrap();

    let radio: LorawanRadio<_, _, MAX_TX_POWER> = lora.into();
    let mut au915 = AU915::new();
    au915.set_join_bias(Subband::_1);
    let region: region::Configuration = au915.into();
    let mut device: Device<_, DefaultFactory, _, _> = Device::new(region, radio, EmbassyTimer::new(), Rng::new(p.RNG, Irqs));

    info!("=== LoRaWAN Join ===");
    info!("Joining network...");

    let join_mode = JoinMode::OTAA {
        deveui: DevEui::from(DEV_EUI),
        appeui: AppEui::from(APP_EUI),
        appkey: AppKey::from(APP_KEY),
    };

    // Join loop
    #[allow(unused_assignments)]
    let mut join_successful = false;
    let mut attempt = 0u32;
    loop {
        attempt += 1;
        info!("Join attempt #{}...", attempt);
        let join_result = device.join(&join_mode).await;
        match join_result {
            Ok(JoinResponse::JoinSuccess) => {
                info!("✓ LoRaWAN network joined successfully!");
                join_successful = true;
                info!("DEBUG: join_successful set to {}", join_successful);
                break;
            }
            Ok(JoinResponse::NoJoinAccept) => {
                warn!("✗ Join failed: No join accept received");
            }
            Err(e) => {
                warn!("✗ Join error: {:?}", e);
            }
        }
        info!("Retrying in 5 seconds...");
        Timer::after_secs(5).await;
    }

    info!("=== Starting sensor loop ===");
    info!("DEBUG: join_successful = {} at start of sensor loop", join_successful);
    
    // Sensor state
    let mut temp_int = 0i16;
    let mut hum_int = 0i16;
    let mut press_int = 0u32;
    let mut gas_resistance = 0u32;
    let mut measurement_failures = 0u8;
    let mut skip_next_measurement = true;
    let mut last_lorawan_op_time: u64 = 0;
    let mut last_data_send = 0u32;
    let mut tx_count = 0u32;
    
    // Note: SNR and RSSI are now tracked by the Device itself
    // We'll get them via device.last_snr() and device.last_rssi()
    
    // Wait longer after join before first sensor read to avoid interference
    // Radio operations can interfere with I2C, so give it time to settle
    // Also reset I2C bus state by doing a dummy transaction
    info!("Waiting for I2C bus to settle after LoRa join...");
    Timer::after_millis(5000).await; // Increased to 5 seconds
    
    // Reset I2C bus state by creating and dropping an I2C instance multiple times
    // This helps recover from any corruption caused by radio interference
    info!("Resetting I2C bus state...");
    for _ in 0..3 {
        let _i2c_reset = unsafe {
            I2c::new_blocking(
                I2C2::steal(),
                PA12::steal(),
                PA11::steal(),
                Hertz(100_000),
                i2c_config.clone(),
            )
        };
        Timer::after_millis(100).await;
        // i2c_reset dropped here, releasing the bus
        Timer::after_millis(100).await;
    }
    Timer::after_millis(500).await; // Final delay

    // Note: Display will be recreated each loop iteration (like working project)
    // This ensures fresh I2C instance each time

    // Text style for display
    let text_style = MonoTextStyle::new(&FONT_6X10, BinaryColor::On);

    loop {
        led.toggle();

        // Skip first measurement after init
        if skip_next_measurement {
            skip_next_measurement = false;
            info!("Skipping measurement (sensor stabilization)");
            Timer::after_secs(2).await;
            continue;
        }
        
        // Wait after LoRaWAN operations to avoid interference (increased delay)
        // Radio operations can cause I2C interference, so wait longer
        let now_ms = embassy_time::Instant::now().as_millis() as u64;
        if last_lorawan_op_time > 0 && now_ms < last_lorawan_op_time + 2000 {
            Timer::after_secs(2).await;
            continue;
        }

        // Read BME688 sensor
        let mut i2c = unsafe {
            I2c::new_blocking(
                I2C2::steal(),
                PA12::steal(),
                PA11::steal(),
                Hertz(100_000),
                i2c_config.clone(),
            )
        };

        // Trigger forced measurement
        let mut ctrl_meas_current = [0u8; 1];
        if i2c.blocking_write_read(bme688_addr, &[BME688_REG_CTRL_MEAS], &mut ctrl_meas_current).is_ok() {
            let sleep_mode = (ctrl_meas_current[0] & 0xFC) | 0x00;
            let _ = i2c.blocking_write(bme688_addr, &[BME688_REG_CTRL_MEAS, sleep_mode]);
            Timer::after_millis(10).await;
        }
        
        if i2c.blocking_write(bme688_addr, &[BME688_REG_CTRL_MEAS, 0x25]).is_ok() {
            Timer::after_millis(500).await; // Wait longer for measurement (humidity needs more time after radio)
            
            let mut data = [0u8; 10];
            if i2c.blocking_write_read(bme688_addr, &[BME688_REG_DATA], &mut data).is_ok() {
                let press_raw = ((data[0] as u32) << 8) | (data[1] as u32);
                let temp_raw = ((data[3] as u32) << 8) | (data[4] as u32);
                let hum_raw = ((data[6] as u32) << 8) | (data[7] as u32);
                let gas_r_msb = data[8] as u32;
                let gas_r_lsb = data[9] as u32;
                let gas_raw = (gas_r_msb << 2) | (gas_r_lsb >> 6);
                
                // Debug: log raw values to see what we're getting
                defmt::debug!("Raw sensor: T={} H={} P={} G={}", temp_raw, hum_raw, press_raw, gas_raw);
                
                // Validation - reject invalid sensor readings
                // BME688 typical ranges:
                // Temp: -40°C to +85°C = raw ~15000 to ~50000 (with 1290 divisor)
                // Hum: 0% to 100% = raw ~0 to ~65535 (with 285 divisor)
                // Press: 300 to 1100 hPa = raw ~5000 to ~40000 (with conversion)
                // Reject common error values: 0x0000, 0x8000 (32768), 0xFFFF (65535)
                let temp_valid = temp_raw > 10000 && temp_raw < 60000 && temp_raw != 32768;
                let hum_valid = hum_raw > 0 && hum_raw < 65535 && hum_raw != 32768; // Reject 0x8000 and 0xFFFF
                let press_valid = press_raw > 3000 && press_raw < 50000 && press_raw != 32768;
                
                defmt::debug!("Validation: T={} H={} P={}", temp_valid, hum_valid, press_valid);
                
                // Accept partial readings - if temp and pressure are valid, use them
                // Humidity may not be ready after radio operations, so keep previous value if invalid
                if temp_valid && press_valid {
                    measurement_failures = 0;
                    
                    if gas_raw == 0x3FF {
                        gas_resistance = 0;
                    } else {
                        gas_resistance = gas_raw;
                    }

                    temp_int = (temp_raw as i32 / 1290) as i16;
                    // Only update humidity if valid, otherwise keep previous value
                    if hum_valid {
                        hum_int = (hum_raw as i32 / 285) as i16;
                        if hum_int > 100 { hum_int = 100; }
                        if hum_int < 0 { hum_int = 0; }
                    }
                    // else: keep previous hum_int value
                    press_int = ((press_raw * 9) / 2) as u32;

                    info!("✓ BME688: {}°C, {}% RH, {} Pa, Gas: {}", 
                          temp_int, hum_int, press_int, gas_resistance);
                    
                    // Update display immediately after successful sensor read
                    // Drop sensor I2C first, then create fresh display instance
                    drop(i2c);
                    // Longer delay to ensure I2C bus is fully released and settled
                    Timer::after_millis(200).await;
                    
                    // Create fresh display instance (like working project)
                    // This ensures clean I2C state after any radio interference
                    let i2c_display = unsafe {
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
                        .connect_i2c(i2c_display)
                        .into();
                    
                    // Init display with error handling
                    // If init fails, it might be due to I2C bus corruption from radio
                    match display.init() {
                        Ok(_) => {
                            // Small delay after init to let display settle
                            Timer::after_millis(50).await;
                            display.clear();
                            // Small delay after clear
                            Timer::after_millis(20).await;
                            
                            // Draw title
                            if Text::new("Node1", Point::new(2, 8), text_style)
                                .draw(&mut display).is_err() {
                                warn!("Failed to draw title");
                            }
                            
                            // Draw sensor data with the values we just read
                            defmt::debug!("Display: T={} H={} P={} Gas={}", temp_int, hum_int, press_int, gas_resistance);
                            
                            let mut temp_hum_buf = heapless::String::<32>::new();
                            if core::fmt::write(&mut temp_hum_buf, format_args!("T:{}C  H:{}%", temp_int, hum_int)).is_ok() {
                                if Text::new(&temp_hum_buf, Point::new(2, 20), text_style)
                                    .draw(&mut display).is_err() {
                                    warn!("Failed to draw temp/hum");
                                }
                            }
                            
                            let press_hpa = press_int / 100;
                            let mut press_buf = heapless::String::<32>::new();
                            if core::fmt::write(&mut press_buf, format_args!("P:{} hPa", press_hpa)).is_ok() {
                                if Text::new(&press_buf, Point::new(2, 32), text_style)
                                    .draw(&mut display).is_err() {
                                    warn!("Failed to draw pressure");
                                }
                            }
                            
                            let mut gas_buf = heapless::String::<32>::new();
                            if gas_resistance > 0 && gas_resistance != 0x3FF {
                                // Show gas as compact value (10-bit raw, 0-1023)
                                let _ = core::fmt::write(&mut gas_buf, format_args!("G:{}", gas_resistance));
                            } else {
                                let _ = core::fmt::write(&mut gas_buf, format_args!("G:--"));
                            }
                            if Text::new(&gas_buf, Point::new(2, 44), text_style)
                                .draw(&mut display).is_err() {
                                warn!("Failed to draw gas");
                            }
                            
                            // Draw LoRaWAN status
                            let mut lora_buf = heapless::String::<24>::new();
                            if join_successful {
                                let snr = device.last_snr();
                                let rssi = device.last_rssi();
                                defmt::debug!("SNR: {}, RSSI: {}, TX: {}", snr, rssi, tx_count);
                                let tx_display = if tx_count > 99 { 99 } else { tx_count };
                                if rssi == 0 {
                                    let _ = core::fmt::write(&mut lora_buf, format_args!("L:J S:-- R:--- T:{}", tx_display));
                                } else {
                                    let _ = core::fmt::write(&mut lora_buf, format_args!("L:J S:{} R:{} T:{}", snr, rssi, tx_display));
                                }
                            } else {
                                let _ = core::fmt::write(&mut lora_buf, format_args!("L:NotJoined"));
                            }
                            if Text::new(&lora_buf, Point::new(2, 56), text_style)
                                .draw(&mut display).is_err() {
                                warn!("Failed to draw LoRa status");
                            }
                            
                            // Small delay before flush to ensure all drawing is complete
                            Timer::after_millis(20).await;
                            
                            // Flush to display with retry (I2C may be corrupted from radio)
                            // Use shorter timeout per attempt to avoid long hangs
                            let mut flush_ok = false;
                            for attempt in 0..5 {
                                match display.flush() {
                                    Ok(_) => {
                                        flush_ok = true;
                                        break;
                                    }
                                    Err(_) => {
                                        if attempt < 4 {
                                            // Longer delay between retries to let I2C recover
                                            Timer::after_millis(100).await;
                                        }
                                    }
                                }
                            }
                            if !flush_ok {
                                warn!("Display flush failed after 5 attempts - I2C may be corrupted");
                                // Display will be recreated next iteration with fresh I2C instance
                            }
                        }
                        Err(_) => {
                            warn!("Failed to init display - I2C bus may be corrupted from radio interference");
                            // Try to recover by waiting longer
                            Timer::after_millis(200).await;
                        }
                    }
                } else {
                    measurement_failures += 1;
                    info!("✗ Invalid sensor data, keeping previous values");
                    // Still drop I2C and update display with previous values
                    drop(i2c);
                }
            } else {
                measurement_failures += 1;
                drop(i2c);
            }
        } else {
            measurement_failures += 1;
            drop(i2c);
        }

        // Reinitialize sensor if too many failures
        if measurement_failures >= 5 {
            info!("Reinitializing sensor...");
            measurement_failures = 0;
            let mut i2c = unsafe {
                I2c::new_blocking(
                    I2C2::steal(),
                    PA12::steal(),
                    PA11::steal(),
                    Hertz(100_000),
                    i2c_config.clone(),
                )
            };
            let _ = i2c.blocking_write(bme688_addr, &[BME688_REG_RESET, BME688_RESET_CMD]);
            Timer::after_millis(10).await;
            let _ = i2c.blocking_write(bme688_addr, &[BME688_REG_CTRL_HUM, 0x01]);
            let _ = i2c.blocking_write(bme688_addr, &[BME688_REG_CTRL_GAS_1, 0x20]);
            let _ = i2c.blocking_write(bme688_addr, &[BME688_REG_RES_HEAT_0, 20]);
            let _ = i2c.blocking_write(bme688_addr, &[BME688_REG_IDAC_HEAT_0, 10]);
            let gas_wait: u8 = (1 << 5) | 25;
            let _ = i2c.blocking_write(bme688_addr, &[BME688_REG_GAS_WAIT_0, gas_wait]);
            let _ = i2c.blocking_write(bme688_addr, &[BME688_REG_CONFIG, 0x00]);
            skip_next_measurement = true;
            Timer::after_millis(500).await;
        }

        // Display update now happens immediately after successful sensor read (see above)
        // Only update display on failure if we haven't already updated it
        if measurement_failures > 0 && measurement_failures < 5 {
            // Update display with previous values (sensor read failed but we have old data)
            // Create fresh display instance (i2c already dropped in failure paths above)
            Timer::after_millis(10).await;
            
            let i2c_display = unsafe {
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
                .connect_i2c(i2c_display)
                .into();
            
            if display.init().is_ok() {
                display.clear();
                
                // Draw title
                if Text::new("Node1", Point::new(2, 8), text_style)
                    .draw(&mut display).is_err() {
                    warn!("Failed to draw title");
                }
                
                // Draw sensor data (previous values)
                let mut temp_hum_buf = heapless::String::<32>::new();
                if core::fmt::write(&mut temp_hum_buf, format_args!("T:{}C  H:{}%", temp_int, hum_int)).is_ok() {
                    if Text::new(&temp_hum_buf, Point::new(2, 20), text_style)
                        .draw(&mut display).is_err() {
                        warn!("Failed to draw temp/hum");
                    }
                }
                
                let press_hpa = press_int / 100;
                let mut press_buf = heapless::String::<32>::new();
                if core::fmt::write(&mut press_buf, format_args!("P:{} hPa", press_hpa)).is_ok() {
                    if Text::new(&press_buf, Point::new(2, 32), text_style)
                        .draw(&mut display).is_err() {
                        warn!("Failed to draw pressure");
                    }
                }
                
            let mut gas_buf = heapless::String::<32>::new();
            if gas_resistance > 0 && gas_resistance != 0x3FF {
                // Show gas as compact value (10-bit raw, 0-1023)
                let _ = core::fmt::write(&mut gas_buf, format_args!("G:{}", gas_resistance));
            } else {
                let _ = core::fmt::write(&mut gas_buf, format_args!("G:--"));
            }
                if Text::new(&gas_buf, Point::new(2, 44), text_style)
                    .draw(&mut display).is_err() {
                    warn!("Failed to draw gas");
                }
                
                // Draw LoRaWAN status
                let mut lora_buf = heapless::String::<24>::new();
                if join_successful {
                    let snr = device.last_snr();
                    let rssi = device.last_rssi();
                    let tx_display = if tx_count > 99 { 99 } else { tx_count };
                    if rssi == 0 {
                        let _ = core::fmt::write(&mut lora_buf, format_args!("L:J S:-- R:--- T:{}", tx_display));
                    } else {
                        let _ = core::fmt::write(&mut lora_buf, format_args!("L:J S:{} R:{} T:{}", snr, rssi, tx_display));
                    }
                } else {
                    let _ = core::fmt::write(&mut lora_buf, format_args!("L:NotJoined"));
                }
                if Text::new(&lora_buf, Point::new(2, 56), text_style)
                    .draw(&mut display).is_err() {
                    warn!("Failed to draw LoRa status");
                }
                
                // Flush to display
                let _ = display.flush();
            } else {
                warn!("Failed to init display");
            }
        }

        // Send sensor data via LoRaWAN every 60 seconds
        if join_successful {
            last_data_send += 1;
            if last_data_send >= 30 {
                info!("Sending sensor data via LoRaWAN...");
                
                let mut payload = [0u8; 4];
                payload[0] = (temp_int + 40) as u8;
                payload[1] = hum_int as u8;
                payload[2] = ((press_int / 100) >> 8) as u8;
                payload[3] = ((press_int / 100) & 0xFF) as u8;
                
                match device.send(&payload, 1, false).await {
                    Ok(response) => {
                        match response {
                            SendResponse::DownlinkReceived(fcnt) => {
                                info!("✓ Data sent, downlink received (FCnt: {})", fcnt);
                                tx_count += 1;
                            }
                            SendResponse::NoAck => {
                                info!("✓ Data sent (no ACK)");
                                tx_count += 1;
                            }
                            SendResponse::RxComplete => {
                                info!("✓ Data sent, RX complete");
                                tx_count += 1;
                            }
                            SendResponse::SessionExpired => {
                                warn!("✗ Session expired, need to rejoin");
                                join_successful = false;
                            }
                        }
                        last_data_send = 0;
                        last_lorawan_op_time = embassy_time::Instant::now().as_millis() as u64;
                    }
                    Err(e) => {
                        warn!("✗ Failed to send data: {:?}", e);
                        last_lorawan_op_time = embassy_time::Instant::now().as_millis() as u64;
                    }
                }
            }
        }

        Timer::after_secs(2).await;
    }
}
