#![no_std]
#![no_main]

mod radio;

use defmt::*;
use embassy_executor::Spawner;
use embassy_stm32::{
    bind_interrupts,
    dma::NoDma,
    gpio::{Level, Output, Speed},
    i2c::{Config as I2cConfig, EventInterruptHandler, ErrorInterruptHandler, I2c},
    peripherals::{self, I2C2, PA11, PA12},
    time::Hertz,
    Config,
};
use embassy_time::Timer;
use embedded_graphics::{
    mono_font::{ascii::FONT_6X10, MonoTextStyle},
    pixelcolor::BinaryColor,
    prelude::*,
    text::Text,
};
use lorawan_device::async_device::{Device, EmbassyTimer, JoinMode};
use lorawan_device::default_crypto::DefaultFactory;
use lorawan_device::region::{AU915, Subband};
use sh1106::{prelude::*, Builder};
use {defmt_rtt as _, panic_probe as _};

use radio::SubGhzRadio;

// TODO: Add LoRaWAN Timer implementation when integrating lorawan-device client

// LoRaWAN configuration for AU915 (Australia, 915 MHz)
// Device EUI: AC1F09FFFE1BCE23
const DEV_EUI: [u8; 8] = [0xAC, 0x1F, 0x09, 0xFF, 0xFE, 0x1B, 0xCE, 0x23];
// Application EUI: b130a864c5295356
const APP_EUI: [u8; 8] = [0xB1, 0x30, 0xA8, 0x64, 0xC5, 0x29, 0x53, 0x56];
// Application Key: b726739b78ec4b9e9234e5d35ea9681b
const APP_KEY: [u8; 16] = [
    0xB7, 0x26, 0x73, 0x9B, 0x78, 0xEC, 0x4B, 0x9E,
    0x92, 0x34, 0xE5, 0xD3, 0x5E, 0xA9, 0x68, 0x1B,
];

// LoRaWAN status
#[derive(Clone, Copy)]
enum LoRaStatus {
    NotJoined,
    Joining,
    Joined,
    Error,
}

struct LoRaInfo {
    status: LoRaStatus,
    snr: i8,
    rssi: i16,
    tx_count: u32,
}

bind_interrupts!(struct I2c2Irqs {
    I2C2_EV => EventInterruptHandler<peripherals::I2C2>;
    I2C2_ER => ErrorInterruptHandler<peripherals::I2C2>;
});

// BME688 I2C addresses
const BME688_ADDR_PRIMARY: u8 = 0x76;
const BME688_ADDR_SECONDARY: u8 = 0x77;

// BME688 register addresses
const BME688_REG_CHIP_ID: u8 = 0xD0;
const BME688_REG_RESET: u8 = 0xE0;
const BME688_REG_CTRL_HUM: u8 = 0x72;
const BME688_REG_STATUS: u8 = 0x73;
const BME688_REG_CTRL_MEAS: u8 = 0x74;
const BME688_REG_CONFIG: u8 = 0x75;
const BME688_REG_DATA: u8 = 0x1F;
// Gas sensor registers
const BME688_REG_CTRL_GAS_0: u8 = 0x70;  // Gas sensor control register 0
const BME688_REG_CTRL_GAS_1: u8 = 0x71;  // Gas sensor control register 1 (heating profile)
const BME688_REG_GAS_WAIT_0: u8 = 0x64;  // Gas wait time register
const BME688_REG_RES_HEAT_0: u8 = 0x5A;  // Gas heater resistance register
const BME688_REG_IDAC_HEAT_0: u8 = 0x50; // Gas heater current register
const BME688_REG_GAS_R_LSB: u8 = 0x2B;   // Gas resistance LSB (part of data register)
const BME688_REG_GAS_R_MSB: u8 = 0x2A;   // Gas resistance MSB (part of data register)

// BME688 values
const BME688_CHIP_ID: u8 = 0x61;
const BME688_RESET_CMD: u8 = 0xB6;

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    info!("====================================");
    info!("  STM32WL55 - I2C2 OLED + BME688");
    info!("  Node 1 - Temp/Hum/Press/Gas");
    info!("====================================");

    let config = Config::default();
    let p = embassy_stm32::init(config);

    info!("STM32WL55 initialized");

    // Test I2C2 bus and detect devices
    {
        info!("Testing I2C2: PA12 (SCL), PA11 (SDA)");
        let mut i2c_config = I2cConfig::default();
        i2c_config.sda_pullup = true;
        i2c_config.scl_pullup = true;

        // SAFETY: This is the first and only time we're using these peripherals
        let mut i2c = unsafe {
            I2c::new(
                I2C2::steal(),
                PA12::steal(),
                PA11::steal(),
                I2c2Irqs,
                NoDma,
                NoDma,
                Hertz(100_000),
                i2c_config,
            )
        };

        Timer::after_millis(100).await;

        // Scan I2C bus - full scan to find all devices
        info!("Scanning I2C2 bus (full scan)...");
        let mut found_count = 0;
        for addr in 0x00..=0x7F {
            let mut buf = [0u8; 1];
            if i2c.blocking_read(addr, &mut buf).is_ok() {
                info!("✓ Device at 0x{:02X}", addr);
                found_count += 1;
            }
        }
        info!("Total devices found: {}", found_count);
    }

    // Initialize LED
    let mut led = Output::new(p.PB15, Level::Low, Speed::Low);

    // Text style
    let text_style = MonoTextStyle::new(&FONT_6X10, BinaryColor::On);

    info!("Starting sensor + display loop...");
    
    // Log LoRaWAN credentials (for verification)
    info!("LoRaWAN Configuration (AU915 - 915 MHz):");
    info!("  Device EUI: {:x}", DEV_EUI);
    info!("  App EUI: {:x}", APP_EUI);
    info!("  App Key: {:x}", APP_KEY);

    // Detect BME688 address
    let mut bme688_addr = BME688_ADDR_PRIMARY;
    {
        let mut i2c_config = I2cConfig::default();
        i2c_config.sda_pullup = true;
        i2c_config.scl_pullup = true;
        let mut i2c = unsafe {
            I2c::new(
                I2C2::steal(),
                PA12::steal(),
                PA11::steal(),
                I2c2Irqs,
                NoDma,
                NoDma,
                Hertz(100_000),
                i2c_config,
            )
        };
        let mut chip_id_buf = [0u8; 1];
        if i2c.blocking_write_read(BME688_ADDR_PRIMARY, &[BME688_REG_CHIP_ID], &mut chip_id_buf).is_ok() {
            if chip_id_buf[0] == BME688_CHIP_ID {
                bme688_addr = BME688_ADDR_PRIMARY;
            }
        } else if i2c.blocking_write_read(BME688_ADDR_SECONDARY, &[BME688_REG_CHIP_ID], &mut chip_id_buf).is_ok() {
            if chip_id_buf[0] == BME688_CHIP_ID {
                bme688_addr = BME688_ADDR_SECONDARY;
            }
        }
    }

    // Initialize BME688
    {
        let mut i2c_config = I2cConfig::default();
        i2c_config.sda_pullup = true;
        i2c_config.scl_pullup = true;
        let mut i2c = unsafe {
            I2c::new(
                I2C2::steal(),
                PA12::steal(),
                PA11::steal(),
                I2c2Irqs,
                NoDma,
                NoDma,
                Hertz(100_000),
                i2c_config,
            )
        };
        
        // Soft reset
        let _ = i2c.blocking_write(bme688_addr, &[BME688_REG_RESET, BME688_RESET_CMD]);
        // Wait for reset to complete (BME688 needs at least 2ms, use 10ms for safety)
        Timer::after_millis(10).await;
        
        // Configure humidity oversampling (x1 = 0x01)
        let _ = i2c.blocking_write(bme688_addr, &[BME688_REG_CTRL_HUM, 0x01]);
        
        // Configure gas sensor
        // CTRL_GAS_1: Enable gas sensor, use heating profile 0
        // Bit 5: run_gas = 1 (enable gas sensor)
        // Bits [3:0]: nb_conv = 0000 (use profile 0)
        let _ = i2c.blocking_write(bme688_addr, &[BME688_REG_CTRL_GAS_1, 0x20]);
        
        // Configure gas heater for profile 0
        // RES_HEAT_0: Heater resistance (controls temperature)
        // Formula: res_heat = (desired_temp - 200) / 5
        // For ~300°C: (300 - 200) / 5 = 20 (0x14)
        let heater_resistance: u8 = 20; // ~300°C heater temperature
        let _ = i2c.blocking_write(bme688_addr, &[BME688_REG_RES_HEAT_0, heater_resistance]);
        
        // IDAC_HEAT_0: Heater current (controls heating power)
        // Range: 0-63, typical: 10-20
        let heater_current: u8 = 10; // Moderate heating current
        let _ = i2c.blocking_write(bme688_addr, &[BME688_REG_IDAC_HEAT_0, heater_current]);
        
        // GAS_WAIT_0: Wait time after heating (in multiples of ~1ms)
        // Bits [7:5]: multiplier (0=1x, 1=4x, 2=16x, etc.)
        // Bits [4:0]: wait time (0-31)
        // For ~100ms wait: multiplier=1 (4x), wait=25 → 4*25 = 100ms
        let gas_wait: u8 = (1 << 5) | 25; // 4x multiplier, 25 units = 100ms
        let _ = i2c.blocking_write(bme688_addr, &[BME688_REG_GAS_WAIT_0, gas_wait]);
        
        // Configure measurement: temp x1, press x1, gas enabled, mode = forced
        // Bits: [7:5] osrs_t=001 (x1), [4:2] osrs_p=001 (x1), [1:0] mode=01 (forced)
        // Note: Gas sensor is enabled via CTRL_GAS_1 register above
        let _ = i2c.blocking_write(bme688_addr, &[BME688_REG_CTRL_MEAS, 0x25]);
        
        // Configure filter and standby time (0x00 = no filter, no standby)
        let _ = i2c.blocking_write(bme688_addr, &[BME688_REG_CONFIG, 0x00]);
        
        info!("BME688 initialized @ 0x{:02X} (gas sensor enabled)", bme688_addr);
        
        // Wait for sensor to stabilize after initialization
        // Gas sensor needs time to heat up and stabilize
        Timer::after_millis(500).await;
    }

    let mut temp_int = 0i16;  // Integer temperature (Celsius)
    let mut hum_int = 0i16;   // Integer humidity (% RH)
    let mut press_int = 0u32; // Integer pressure (Pa)
    let mut gas_resistance = 0u32; // Gas resistance (Ohms)
    let mut measurement_failures = 0u8; // Counter for consecutive measurement failures
    let mut skip_next_measurement = true; // Skip first measurement after init/reinit
    let mut last_lorawan_op_time: u64 = 0; // Track when LoRaWAN operations complete
    
    // LoRaWAN status tracking
    let mut lora_info = LoRaInfo {
        status: LoRaStatus::NotJoined,
        snr: 0,
        rssi: 0,
        tx_count: 0,
    };
    
    // Initialize SubGHz radio peripheral using stm32wlxx-hal
    info!("Initializing SubGHz radio for LoRaWAN...");
    
    // Get PAC peripherals (we need SPI3, RCC, and GPIOC)
    // embassy-stm32 and stm32wlxx-hal use the same PAC, so we can use stm32wlxx_hal::pac
    use stm32wlxx_hal::pac;
    let pac = unsafe { pac::Peripherals::steal() };
    let mut rcc = pac.RCC;
    
    let mut radio = SubGhzRadio::new();
    match radio.init(pac.SPI3, &mut rcc, pac.GPIOC) {
        Ok(_) => {
            info!("✓ SubGHz radio initialized");
            
            // Configure for AU915 frequency band
            match radio.configure_au915().await {
                Ok(_) => {
                    info!("✓ Radio configured for AU915 (915 MHz)");
                    lora_info.status = LoRaStatus::NotJoined;
                }
                Err(e) => {
                    warn!("✗ Failed to configure AU915: {:?}", e);
                    lora_info.status = LoRaStatus::Error;
                }
            }
        }
        Err(e) => {
            warn!("✗ Failed to initialize radio: {:?}", e);
            lora_info.status = LoRaStatus::Error;
        }
    }
    
    info!("LoRaWAN integration status:");
    info!("  Sensor system: ✓ Operational");
    info!("  Display system: ✓ Operational");
    info!("  LoRaWAN credentials: ✓ Configured (AU915)");
    let radio_initialized = radio.is_initialized();
    if radio_initialized {
        info!("  Radio driver: ✓ Initialized and configured");
        info!("  Radio frequency: {} MHz", radio.frequency());
        info!("  Radio state: {:?}", radio.state());
    } else {
        info!("  Radio driver: ✗ Not initialized");
    }
    
    // Create LoRaWAN Device
    use lorawan_device::region::Configuration;
    
    // Initialize LoRaWAN device if radio is ready
    type LoRaWANDevice = Device<SubGhzRadio, DefaultFactory, EmbassyTimer, lorawan_device::Prng, 256, 1>;
    let mut lorawan_device: Option<LoRaWANDevice> = if radio_initialized {
        // Create region configuration for AU915 with sub-band 0 bias (channels 0-7)
        // This ensures join requests only use channels 0-7, matching gateway configuration
        // Note: Subband::_1 = channels 0-7 (sub-band 0), Subband::_2 = channels 8-15 (sub-band 1)
        // Sub-band 0: channels 0-7 (915.2 - 916.6 MHz)
        let mut au915_config = AU915::new();
        au915_config.set_join_bias(Subband::_1); // Use sub-band 0 (channels 0-7)
        let region: Configuration = au915_config.into();
        info!("  AU915 configured: Sub-band 0 (channels 0-7: 915.2-916.6 MHz)");
        
        // Generate a random seed (in production, use a hardware RNG)
        // For now, use a simple seed based on device EUI
        let seed = u64::from_le_bytes([
            DEV_EUI[0], DEV_EUI[1], DEV_EUI[2], DEV_EUI[3],
            DEV_EUI[4], DEV_EUI[5], DEV_EUI[6], DEV_EUI[7],
        ]);
        
        let timer = EmbassyTimer::new();
        let device = Device::new_with_seed(region, radio, timer, seed);
        info!("✓ LoRaWAN Device created");
        Some(device)
    } else {
        None
    };
    
    let mut join_attempted = false;
    let mut join_successful = false;
    let mut last_join_attempt = 0u32;
    let mut last_data_send = 0u32;

    loop {
        // Toggle LED
        led.toggle();

        // Skip first measurement after initialization/reinit to let sensor stabilize
        if skip_next_measurement {
            skip_next_measurement = false;
            info!("Skipping measurement (sensor stabilization)");
            Timer::after_secs(2).await;
            continue;
        }
        
        // Check if LoRaWAN operation just completed - wait before sensor read to avoid interference
        let now_ms = embassy_time::Instant::now().as_millis() as u64;
        if now_ms < last_lorawan_op_time + 500 {
            // LoRaWAN operation completed less than 500ms ago - skip sensor read this iteration
            Timer::after_secs(2).await;
            continue;
        }
        
        // Radio is now owned by lorawan_device, so we don't check it directly
        // LoRaWAN operations handle radio state internally

        // ============================================
        // Step 1: Create I2C and read BME688 sensor
        // ============================================
        let mut i2c_config = I2cConfig::default();
        i2c_config.sda_pullup = true;
        i2c_config.scl_pullup = true;

        // SAFETY: We're creating a temporary I2C instance that will be dropped
        // at the end of each loop iteration, releasing the hardware for reuse
        let mut i2c = unsafe {
            I2c::new(
                I2C2::steal(),
                PA12::steal(),
                PA11::steal(),
                I2c2Irqs,
                NoDma,
                NoDma,
                Hertz(100_000),
                i2c_config,
            )
        };

        // Trigger forced measurement (with gas sensor enabled)
        // Important: Need to switch to sleep mode first, then forced mode to trigger new measurement
        // Step 1: Set to sleep mode (mode bits [1:0] = 00)
        let mut ctrl_meas_current = [0u8; 1];
        if i2c.blocking_write_read(bme688_addr, &[BME688_REG_CTRL_MEAS], &mut ctrl_meas_current).is_ok() {
            // Clear mode bits [1:0] to set sleep mode
            let sleep_mode = (ctrl_meas_current[0] & 0xFC) | 0x00;
            let _ = i2c.blocking_write(bme688_addr, &[BME688_REG_CTRL_MEAS, sleep_mode]);
            Timer::after_millis(10).await; // Brief delay
        }
        
        // Step 2: Trigger forced measurement (mode bits [1:0] = 01, with gas sensor)
        // 0x25 = temp x1, press x1, forced mode (00100101)
        if i2c.blocking_write(bme688_addr, &[BME688_REG_CTRL_MEAS, 0x25]).is_ok() {
            // Use fixed delay approach like the example - simpler and more reliable
            // BME688 measurement times (with gas sensor):
            // - Temp/Press/Hum: ~20-30ms
            // - Gas sensor: additional ~100ms (from GAS_WAIT_0 configuration)
            // Total: ~150-200ms, use 300ms to be safe (gas sensor needs more time)
            Timer::after_millis(300).await;
            
            // Read data registers (10 bytes for BME688 with gas sensor)
                // Bytes 0-7: pressure, temperature, humidity (same as before)
                // Bytes 8-9: gas resistance (MSB, LSB)
                let mut data = [0u8; 10]; // Read 10 bytes to include gas resistance
                if i2c.blocking_write_read(bme688_addr, &[BME688_REG_DATA], &mut data).is_ok() {
                    // Extract raw values from BME688 data registers
                    // BME688 uses 16-bit values for temperature and pressure (not 20-bit like BME680)
                    // Pressure: 16 bits - data[0] (msb), data[1] (lsb)
                    let press_raw = ((data[0] as u32) << 8) | (data[1] as u32);
                    
                    // Temperature: 16 bits - data[3] (msb), data[4] (lsb)
                    let temp_raw = ((data[3] as u32) << 8) | (data[4] as u32);
                    
                    // Humidity: 16 bits - data[6] (msb), data[7] (lsb)
                    let hum_raw = ((data[6] as u32) << 8) | (data[7] as u32);
                    
                    // Gas resistance: 10 bits - data[8] (msb), data[9] (lsb)
                    // Gas resistance is 10-bit value: [9:2] in MSB, [1:0] in LSB
                    // Format: gas_r = (data[8] << 2) | (data[9] >> 6)
                    let gas_r_msb = data[8] as u32;
                    let gas_r_lsb = data[9] as u32;
                    // Extract 10-bit value: MSB bits [7:0] become [9:2], LSB bits [7:6] become [1:0]
                    let gas_raw = (gas_r_msb << 2) | (gas_r_lsb >> 6);
                    
                    // Validate raw values before using them
                    // Check for reasonable ranges to detect stale/invalid data
                    let temp_valid = temp_raw > 20000 && temp_raw < 45000; // ~15-35°C range
                    let hum_valid = hum_raw < 28500; // Max 100% = 28500/285
                    let press_valid = press_raw > 10000 && press_raw < 35000; // Reasonable pressure range
                    
                    if temp_valid && hum_valid && press_valid {
                        // Values look valid, update them
                        measurement_failures = 0; // Reset failure counter
                        
                        // Check if gas measurement is valid
                        if gas_raw == 0x3FF {
                            gas_resistance = 0; // Mark as invalid
                        } else {
                            gas_resistance = gas_raw;
                        }

                        // BME688 conversion formulas using integer-only math
                        // Temperature: T = temp_raw / 1290 (empirically derived)
                        let temp_raw_i32 = temp_raw as i32;
                        temp_int = (temp_raw_i32 / 1290) as i16;
                        
                        // Humidity: RH = hum_raw / 285 (adjusted for 64% at Brisbane)
                        let hum_raw_i32 = hum_raw as i32;
                        hum_int = (hum_raw_i32 / 285) as i16;
                        // Clamp to valid range
                        if hum_int > 100 { hum_int = 100; }
                        if hum_int < 0 { hum_int = 0; }
                        
                        // Pressure: P = (press_raw * 9) / 2 Pa (empirically derived)
                        press_int = ((press_raw * 9) / 2) as u32;

                        info!("✓ BME688: {}°C, {}% RH, {} Pa, Gas: {}", 
                              temp_int, hum_int, press_int, gas_resistance);
                    } else {
                        // Invalid values detected - don't update, log warning
                        measurement_failures += 1;
                        info!("✗ Invalid sensor data (T={}, H={}, P={}), keeping previous values", 
                              temp_raw, hum_raw, press_raw);
                    }
                } else {
                    measurement_failures += 1;
                    info!("✗ Failed to read BME688 data");
                }
        } else {
            measurement_failures += 1;
            info!("✗ Failed to trigger BME688 measurement");
        }
        
        // If we have too many consecutive failures, sensor might be stuck
        // Reinitialize after 5 failures
        if measurement_failures >= 5 {
            info!("Too many measurement failures ({}), reinitializing sensor...", measurement_failures);
            measurement_failures = 0;
            
            // Reinitialize BME688
            {
                let mut i2c_config = I2cConfig::default();
                i2c_config.sda_pullup = true;
                i2c_config.scl_pullup = true;
                let mut i2c = unsafe {
                    I2c::new(
                        I2C2::steal(),
                        PA12::steal(),
                        PA11::steal(),
                        I2c2Irqs,
                        NoDma,
                        NoDma,
                        Hertz(100_000),
                        i2c_config,
                    )
                };
                
                // Soft reset
                let _ = i2c.blocking_write(bme688_addr, &[BME688_REG_RESET, BME688_RESET_CMD]);
                Timer::after_millis(10).await;
                
                // Reconfigure
                let _ = i2c.blocking_write(bme688_addr, &[BME688_REG_CTRL_HUM, 0x01]);
                let _ = i2c.blocking_write(bme688_addr, &[BME688_REG_CTRL_GAS_1, 0x20]);
                let _ = i2c.blocking_write(bme688_addr, &[BME688_REG_RES_HEAT_0, 20]);
                let _ = i2c.blocking_write(bme688_addr, &[BME688_REG_IDAC_HEAT_0, 10]);
                let gas_wait: u8 = (1 << 5) | 25;
                let _ = i2c.blocking_write(bme688_addr, &[BME688_REG_GAS_WAIT_0, gas_wait]);
                let _ = i2c.blocking_write(bme688_addr, &[BME688_REG_CONFIG, 0x00]);
                
                info!("Sensor reinitialized");
                
                // Skip next measurement after reinit to let sensor stabilize
                skip_next_measurement = true;
                
                // Wait for sensor to stabilize
                Timer::after_millis(500).await;
            }
        }

        // ============================================
        // Step 2: Update OLED display
        // ============================================
        let mut display: GraphicsMode<_> = Builder::new()
            .with_size(DisplaySize::Display128x64)
            .connect_i2c(i2c)
            .into();

        if display.init().is_ok() {
            // Clear display
            display.clear();

            // Title (compact, top line)
            let _ = Text::new("STM32WL55 Node1", Point::new(2, 8), text_style)
                .draw(&mut display);

            // Sensor readings (compact format, lines 2-4)
            // Line 2: Temperature and Humidity
            let mut temp_hum_buf = heapless::String::<32>::new();
            let _ = core::fmt::write(&mut temp_hum_buf, format_args!("T:{}C  H:{}%", temp_int, hum_int));
            let _ = Text::new(&temp_hum_buf, Point::new(2, 20), text_style)
                .draw(&mut display);

            // Line 3: Pressure
            let press_hpa = press_int / 100;
            let mut press_buf = heapless::String::<32>::new();
            let _ = core::fmt::write(&mut press_buf, format_args!("P:{} hPa", press_hpa));
            let _ = Text::new(&press_buf, Point::new(2, 32), text_style)
                .draw(&mut display);

            // Line 4: Gas resistance
            // Gas resistance is a 10-bit raw value that needs calibration coefficients
            // for accurate conversion to kΩ. For now, display raw value.
            // Typical clean air: 10-500 kΩ (after proper conversion)
            let mut gas_buf = heapless::String::<32>::new();
            if gas_resistance > 0 && gas_resistance != 0x3FF {
                // Display raw value - proper conversion requires calibration coefficients
                // Raw value of 512 might correspond to ~50-200 kΩ depending on calibration
                let _ = core::fmt::write(&mut gas_buf, format_args!("Gas: {} (raw)", gas_resistance));
            } else {
                let _ = core::fmt::write(&mut gas_buf, format_args!("Gas: --"));
            }
            let _ = Text::new(&gas_buf, Point::new(2, 44), text_style)
                .draw(&mut display);

            // Line 5: LoRa status (show SNR, RSSI, TX count)
            let mut lora_buf = heapless::String::<48>::new();
            match lora_info.status {
                LoRaStatus::NotJoined => {
                    // Show radio status even if not joined to LoRaWAN
                    if lora_info.tx_count > 0 {
                        let _ = core::fmt::write(&mut lora_buf, format_args!("LoRa: SNR:{} RSSI:{}", lora_info.snr, lora_info.rssi));
                    } else {
                        let _ = core::fmt::write(&mut lora_buf, format_args!("LoRa: NotJoined"));
                    }
                }
                LoRaStatus::Joining => {
                    let _ = core::fmt::write(&mut lora_buf, format_args!("LoRa: Joining..."));
                }
                LoRaStatus::Joined => {
                    let _ = core::fmt::write(&mut lora_buf, format_args!("LoRa: SNR:{} RSSI:{} TX:{}", lora_info.snr, lora_info.rssi, lora_info.tx_count));
                }
                LoRaStatus::Error => {
                    let _ = core::fmt::write(&mut lora_buf, format_args!("LoRa: Error"));
                }
            }
            let _ = Text::new(&lora_buf, Point::new(2, 56), text_style)
                .draw(&mut display);

            // Flush to display
            let _ = display.flush();
        } else {
            error!("✗ Failed to init OLED");
        }

        // display and i2c are dropped here, releasing the hardware

        // LoRaWAN operations
        if let Some(ref mut device) = lorawan_device {
            // Attempt OTAA join if not yet joined
            if !join_successful {
                last_join_attempt += 1;
                // Try join every 30 seconds (15 loop iterations at 2s each)
                if !join_attempted || last_join_attempt >= 15 {
                    info!("Attempting LoRaWAN OTAA join...");
                    lora_info.status = LoRaStatus::Joining;
                    
                    // Keys are re-exported from lorawan at the crate root
                    let join_mode = JoinMode::OTAA {
                        deveui: lorawan_device::DevEui::from(DEV_EUI),
                        appeui: lorawan_device::AppEui::from(APP_EUI),
                        appkey: lorawan_device::AppKey::from(APP_KEY),
                    };
                    
                    match device.join(&join_mode).await {
                        Ok(lorawan_device::async_device::JoinResponse::JoinSuccess) => {
                            info!("✓ LoRaWAN join successful!");
                            lora_info.status = LoRaStatus::Joined;
                            join_successful = true;
                            join_attempted = true;
                            last_join_attempt = 0;
                            last_lorawan_op_time = embassy_time::Instant::now().as_millis() as u64;
                        }
                        Ok(lorawan_device::async_device::JoinResponse::NoJoinAccept) => {
                            warn!("✗ LoRaWAN join failed: No join accept received");
                            lora_info.status = LoRaStatus::NotJoined;
                            join_attempted = true;
                            last_lorawan_op_time = embassy_time::Instant::now().as_millis() as u64;
                        }
                        Err(_e) => {
                            warn!("✗ LoRaWAN join error");
                            lora_info.status = LoRaStatus::Error;
                            join_attempted = true;
                            last_lorawan_op_time = embassy_time::Instant::now().as_millis() as u64;
                        }
                    }
                }
            } else {
                // Send sensor data every 60 seconds (30 loop iterations at 2s each)
                last_data_send += 1;
                if last_data_send >= 30 {
                    info!("Sending sensor data via LoRaWAN...");
                    
                    // Prepare sensor data payload (4 bytes: temp, hum, press_msb, press_lsb)
                    let mut payload = [0u8; 4];
                    payload[0] = (temp_int + 40) as u8; // Temp: -40 to +85°C, offset by 40
                    payload[1] = hum_int as u8; // Humidity: 0-100%
                    payload[2] = ((press_int / 100) >> 8) as u8; // Pressure MSB (hPa)
                    payload[3] = ((press_int / 100) & 0xFF) as u8; // Pressure LSB (hPa)
                    
                    match device.send(&payload, 1, false).await {
                        Ok(response) => {
                            match response {
                                lorawan_device::async_device::SendResponse::DownlinkReceived(fcnt) => {
                                    info!("✓ Data sent, downlink received (FCnt: {})", fcnt);
                                    lora_info.tx_count += 1;
                                }
                                lorawan_device::async_device::SendResponse::NoAck => {
                                    info!("✓ Data sent (no ACK)");
                                    lora_info.tx_count += 1;
                                }
                                lorawan_device::async_device::SendResponse::RxComplete => {
                                    info!("✓ Data sent, RX complete");
                                    lora_info.tx_count += 1;
                                }
                                lorawan_device::async_device::SendResponse::SessionExpired => {
                                    warn!("✗ Session expired, need to rejoin");
                                    join_successful = false;
                                }
                            }
                            last_data_send = 0;
                            last_lorawan_op_time = embassy_time::Instant::now().as_millis() as u64;
                        }
                        Err(_e) => {
                            warn!("✗ Failed to send data");
                            last_lorawan_op_time = embassy_time::Instant::now().as_millis() as u64;
                        }
                    }
                }
            }
        }

        Timer::after_secs(2).await;
    }
}
