//! STM32WL55 Sensor Node with BME680/BME688 and OLED Display
//! Note: BME680 and BME688 are compatible (same chip ID 0x61)
#![no_std]
#![no_main]

mod iv;

use defmt::*;
use embassy_executor::Spawner;
use embassy_stm32::gpio::{Level, Output, Pin, Speed};
use embassy_stm32::i2c::{Config as I2cConfig, ErrorInterruptHandler, EventInterruptHandler, I2c};
use embassy_stm32::peripherals::{I2C2, PA11, PA12};
use embassy_stm32::rcc::*;
use embassy_stm32::rng::{self, Rng};
use embassy_stm32::spi::Spi;
use embassy_stm32::time::Hertz;
use embassy_stm32::{bind_interrupts, peripherals};
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
use lorawan_device::async_device::{Device, EmbassyTimer, JoinMode, JoinResponse, SendResponse};
use lorawan_device::default_crypto::DefaultFactory;
use lorawan_device::region::{AU915, Subband, Configuration};
use lorawan_device::{AppEui, AppKey, DevEui};
use sh1106::{prelude::*, Builder};
use {defmt_rtt as _, panic_probe as _};

use self::iv::{InterruptHandler, Stm32wlInterfaceVariant, SubghzSpiDevice};

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
const BME688_REG_STATUS: u8 = 0x73;
const BME688_CHIP_ID: u8 = 0x61;
const BME688_RESET_CMD: u8 = 0xB6;

// Calibration coefficient registers (BME680/BME688 compatible)
const BME688_REG_CALIB00: u8 = 0x89; // Temperature calibration start
const BME688_REG_CALIB25: u8 = 0xA1; // Temperature calibration end
const BME688_REG_CALIB26: u8 = 0xE1; // Pressure/Humidity calibration start
const BME688_REG_CALIB41: u8 = 0xF0; // Pressure/Humidity calibration end

// Calibration data structure
struct BmeCalibration {
    // Temperature calibration
    par_t1: u16,
    par_t2: i16,
    par_t3: i8,
    // Pressure calibration
    par_p1: u16,
    par_p2: i16,
    par_p3: i8,
    par_p4: i16,
    par_p5: i16,
    par_p6: i8,
    par_p7: i8,
    par_p8: i16,
    par_p9: i16,
    par_p10: u8,
    // Humidity calibration
    par_h1: u16,
    par_h2: u16,
    par_h3: i8,
    par_h4: i8,
    par_h5: i8,
    par_h6: u8,
    par_h7: i8,
}

impl BmeCalibration {
    fn new() -> Self {
        Self {
            par_t1: 0,
            par_t2: 0,
            par_t3: 0,
            par_p1: 0,
            par_p2: 0,
            par_p3: 0,
            par_p4: 0,
            par_p5: 0,
            par_p6: 0,
            par_p7: 0,
            par_p8: 0,
            par_p9: 0,
            par_p10: 0,
            par_h1: 0,
            par_h2: 0,
            par_h3: 0,
            par_h4: 0,
            par_h5: 0,
            par_h6: 0,
            par_h7: 0,
        }
    }

    // Read calibration coefficients from sensor
    fn read_calibration<M: embassy_stm32::mode::Mode>(&mut self, i2c: &mut I2c<'_, M>, addr: u8) -> bool {
        let mut calib_data = [0u8; 41];
        
        // Read temperature calibration (0x89-0xA1, 25 bytes)
        if i2c.blocking_write_read(addr, &[BME688_REG_CALIB00], &mut calib_data[0..25]).is_err() {
            return false;
        }
        
        // Read pressure/humidity calibration (0xE1-0xF0, 16 bytes)
        if i2c.blocking_write_read(addr, &[BME688_REG_CALIB26], &mut calib_data[25..41]).is_err() {
            return false;
        }
        
        // Parse temperature coefficients
        self.par_t1 = (calib_data[33] as u16) | ((calib_data[34] as u16) << 8);
        self.par_t2 = (calib_data[0] as i16) | ((calib_data[1] as i16) << 8);
        self.par_t3 = calib_data[2] as i8;
        
        // Parse pressure coefficients
        self.par_p1 = (calib_data[6] as u16) | ((calib_data[7] as u16) << 8);
        self.par_p2 = (calib_data[8] as i16) | ((calib_data[9] as i16) << 8);
        self.par_p3 = calib_data[10] as i8;
        self.par_p4 = (calib_data[12] as i16) | ((calib_data[13] as i16) << 8);
        self.par_p5 = (calib_data[14] as i16) | ((calib_data[15] as i16) << 8);
        self.par_p6 = calib_data[16] as i8;
        self.par_p7 = calib_data[17] as i8;
        self.par_p8 = (calib_data[18] as i16) | ((calib_data[19] as i16) << 8);
        self.par_p9 = (calib_data[20] as i16) | ((calib_data[21] as i16) << 8);
        self.par_p10 = calib_data[22];
        
        // Parse humidity coefficients
        self.par_h1 = ((calib_data[25] as u16) << 4) | (calib_data[26] as u16 & 0x0F);
        self.par_h2 = ((calib_data[26] as u16) >> 4) | ((calib_data[27] as u16) << 4);
        self.par_h3 = calib_data[28] as i8;
        self.par_h4 = calib_data[29] as i8;
        self.par_h5 = calib_data[30] as i8;
        self.par_h6 = calib_data[31];
        self.par_h7 = calib_data[32] as i8;
        
        true
    }

    // Compensate temperature (returns temperature in 0.01°C, e.g., 2500 = 25.00°C)
    fn compensate_temperature(&self, temp_raw: u32) -> i32 {
        let t_fine = self.calculate_t_fine(temp_raw);
        (((t_fine * 5 + 128) >> 8) / 100) as i32
    }
    
    // Calculate t_fine (needed for pressure and humidity compensation)
    fn calculate_t_fine(&self, temp_raw: u32) -> i64 {
        let var1 = ((temp_raw as i64 >> 3) - ((self.par_t1 as i64) << 1)) * (self.par_t2 as i64);
        let var2 = ((((temp_raw as i64 >> 4) - (self.par_t1 as i64)) * 
                     ((temp_raw as i64 >> 4) - (self.par_t1 as i64))) >> 12) * 
                    ((self.par_t3 as i64) << 4);
        var1 + var2
    }

    // Compensate pressure (returns pressure in Pa)
    fn compensate_pressure(&self, press_raw: u32, t_fine: i64) -> u32 {
        let var1 = ((t_fine >> 1) - 64000) as i64;
        let var2 = (((var1 >> 2) * (var1 >> 2)) >> 11) * (self.par_p6 as i64);
        let var2 = var2 + ((var1 * (self.par_p5 as i64)) << 1);
        let var2 = (var2 >> 2) + ((self.par_p4 as i64) << 16);
        let var1 = (((((var1 >> 2) * (var1 >> 2)) >> 13) * ((self.par_p3 as i64) << 3)) >> 3) + 
                   (((self.par_p2 as i64) * var1) >> 1);
        let var1 = var1 >> 18;
        let var1 = ((32768 + var1) * (self.par_p1 as i64)) >> 15;
        if var1 == 0 {
            return 0;
        }
        let pressure = ((1048576 - (press_raw as i64)) - (var2 >> 12)) * 3125;
        let pressure = if pressure < 0x80000000 {
            (pressure << 1) / var1
        } else {
            (pressure / var1) << 1
        };
        let var1 = ((self.par_p9 as i64) * (((pressure >> 3) * (pressure >> 3)) >> 13)) >> 12;
        let var2 = (((pressure >> 2) * (self.par_p8 as i64)) >> 13) as i64;
        let var3 = ((((pressure >> 8) * (pressure >> 8) * (pressure >> 8) * (self.par_p10 as i64)) >> 3) >> 13) as i64;
        let pressure = (pressure + ((var1 + var2 + var3 + ((self.par_p7 as i64) << 7)) >> 4)) as u32;
        pressure
    }

    // Compensate humidity (returns humidity in %RH * 100, e.g., 6400 = 64.00%)
    fn compensate_humidity(&self, hum_raw: u32, t_fine: i64) -> u32 {
        // Use i64 throughout to prevent overflow, use wrapping operations to avoid panics
        let var1 = (hum_raw as i64) - ((self.par_h1 as i64) << 4) - 
                   (((t_fine.wrapping_mul(self.par_h3 as i64)) / 100) >> 1);
        let var2_temp = ((t_fine.wrapping_mul(self.par_h4 as i64)) / 100) + 
                        (((t_fine.wrapping_mul((t_fine.wrapping_mul(self.par_h5 as i64)) / 100)) >> 6) / 100) + (1i64 << 14);
        let var2 = ((self.par_h2 as i64).wrapping_mul(var2_temp)) >> 10;
        let var3 = var1.wrapping_mul(var2);
        let var4 = (((self.par_h6 as i64) << 7) + ((t_fine.wrapping_mul(self.par_h7 as i64)) / 100)) >> 4;
        let var3_shifted = var3 >> 14;
        let var5 = (var3_shifted.wrapping_mul(var3_shifted)) >> 10;
        let var6 = (var4.wrapping_mul(var5)) >> 1;
        let humidity = ((var3 + var6) >> 12) * 1000 / 1024;
        humidity as u32
    }
}

// LoRaWAN configuration for AU915 (Australia, 915 MHz)
const DEV_EUI: [u8; 8] = [0xAC, 0x1F, 0x09, 0xFF, 0xFE, 0x1B, 0xCE, 0x23];
const APP_EUI: [u8; 8] = [0xB1, 0x30, 0xA8, 0x64, 0xC5, 0x29, 0x53, 0x56];
const APP_KEY: [u8; 16] = [
    0xB7, 0x26, 0x73, 0x9B, 0x78, 0xEC, 0x4B, 0x9E,
    0x92, 0x34, 0xE5, 0xD3, 0x5E, 0xA9, 0x68, 0x1B,
];
const MAX_TX_POWER: u8 = 14;

bind_interrupts!(struct Irqs {
    SUBGHZ_RADIO => InterruptHandler;
    RNG => rng::InterruptHandler<peripherals::RNG>;
    I2C2_EV => EventInterruptHandler<peripherals::I2C2>;
    I2C2_ER => ErrorInterruptHandler<peripherals::I2C2>;
});

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    info!("====================================");
    info!("  STM32WL55 - Sensor Node");
    info!("  BME680/BME688 + OLED + LoRaWAN");
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

    // Initialize LoRaWAN radio
    info!("Initializing LoRaWAN radio...");
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
    let region: Configuration = au915.into();
    let mut device: Device<_, DefaultFactory, _, _> = Device::new(region, radio, EmbassyTimer::new(), Rng::new(p.RNG, Irqs));
    info!("✓ LoRaWAN radio initialized");

    // LoRaWAN state
    let mut join_successful = false;
    let mut last_data_send = 0u32;
    let mut tx_count = 0u32;
    let mut last_snr: i8 = 0;
    let mut last_rssi: i16 = 0;

    // Initialize I2C config
    let mut i2c_config = I2cConfig::default();
    i2c_config.sda_pullup = true;
    i2c_config.scl_pullup = true;

    // Scan I2C bus to find BME680/BME688
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
                    info!("✓ BME680/BME688 found at 0x{:02X}", addr);
                    break;
                }
            }
        }
    }

    // Initialize BME680/BME688 sensor
    info!("Initializing BME680/BME688...");
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

        // Configure gas sensor (like reference: 150ms heating, 300°C, profile 0)
        // Reference uses: .with_gas_measurement(Duration::from_millis(150), 300, 25)
        // CTRL_GAS_1: Bit 5 (run_gas) = 1 to enable, bits [3:0] = profile number
        let _ = i2c.blocking_write(bme688_addr, &[BME688_REG_CTRL_GAS_1, 0x20]); // Enable gas (bit 5=1), profile 0
        // Heater resistance for ~300°C: (300 - 200) / 5 = 20
        let _ = i2c.blocking_write(bme688_addr, &[BME688_REG_RES_HEAT_0, 20]);
        // Heater current (IDAC): moderate value (10)
        let _ = i2c.blocking_write(bme688_addr, &[BME688_REG_IDAC_HEAT_0, 10]);
        // Gas wait time: multiplier=1 (4x), wait=38 → 4*38 = 152ms (close to reference 150ms)
        let gas_wait: u8 = (1 << 5) | 38; // 4x multiplier, 38 units = ~152ms
        let _ = i2c.blocking_write(bme688_addr, &[BME688_REG_GAS_WAIT_0, gas_wait]);
        // Verify gas sensor is enabled by reading back CTRL_GAS_1
        let mut ctrl_gas1_check = [0u8; 1];
        if i2c.blocking_write_read(bme688_addr, &[BME688_REG_CTRL_GAS_1], &mut ctrl_gas1_check).is_ok() {
            if (ctrl_gas1_check[0] & 0x20) == 0 {
                warn!("⚠ Gas sensor not enabled! CTRL_GAS_1=0x{:02X}", ctrl_gas1_check[0]);
            } else {
                defmt::debug!("Gas sensor enabled: CTRL_GAS_1=0x{:02X}", ctrl_gas1_check[0]);
            }
        }
        let _ = i2c.blocking_write(bme688_addr, &[BME688_REG_CONFIG, 0x00]);

        info!("✓ BME680/BME688 initialized");
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
    let mut temp_tenths = 0i16; // Temperature in tenths of a degree (e.g., 245 = 24.5°C)
    let mut hum_tenths = 0i16; // Humidity in tenths of a percent (e.g., 815 = 81.5%)
    let mut press_int = 0u32;
    let mut gas_resistance = 0u32;
    
    // Calibration disabled for now - coefficient parsing needs to be verified for BME688
    // Using simplified formulas that were working before

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
            warn!("✗ BME680/BME688 not responding (chip ID: 0x{:02X})", chip_id_buf[0]);
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

        // Set sleep mode first (required before triggering forced mode)
        // Like reference: just set mode, don't reconfigure gas settings (they're set at startup)
        let sleep_mode = (ctrl_meas[0] & 0xFC) | 0x00;
        if i2c.blocking_write(bme688_addr, &[BME688_REG_CTRL_MEAS, sleep_mode]).is_err() {
            warn!("✗ Failed to set sleep mode");
            drop(i2c);
            Timer::after_secs(2).await;
            continue;
        }

        Timer::after_millis(20).await;

        // Trigger forced measurement: 0x25 = temp x1, press x1, forced mode (bits [1:0] = 01)
        // Gas sensor is already enabled via CTRL_GAS_1 at startup
        // This is the key step: writing to CTRL_MEAS with mode=01 triggers the measurement
        if i2c.blocking_write(bme688_addr, &[BME688_REG_CTRL_MEAS, 0x25]).is_err() {
            warn!("✗ Failed to trigger measurement");
            drop(i2c);
            Timer::after_secs(2).await;
            continue;
        }
        
        // Small delay to ensure the mode change takes effect
        Timer::after_millis(10).await;
        
        // Verify the mode was set correctly (optional, for debugging)
        let mut ctrl_meas_check = [0u8; 1];
        if i2c.blocking_write_read(bme688_addr, &[BME688_REG_CTRL_MEAS], &mut ctrl_meas_check).is_ok() {
            if (ctrl_meas_check[0] & 0x03) != 0x01 {
                warn!("⚠ Forced mode not set correctly: CTRL_MEAS=0x{:02X}", ctrl_meas_check[0]);
            }
        }

        // Wait for measurement completion (like reference: 2000ms for gas sensor)
        // The sensor performs TPHG (Temperature, Pressure, Humidity, Gas) measurement
        // Gas sensor heating takes ~150ms, total measurement ~2000ms
        // Use fixed delay approach - simpler and more reliable (matches reference)
        Timer::after_millis(2000).await;

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
            // Gas resistance extraction (BME680/BME688 format)
            // Gas resistance is stored as: gas_r_msb (8 bits) and gas_r_lsb (2 bits in upper bits)
            // Format: gas_r = (gas_r_msb << 2) | (gas_r_lsb >> 6)
            let gas_r_msb = data[8] as u32;
            let gas_r_lsb = data[9] as u32;
            let gas_raw = (gas_r_msb << 2) | (gas_r_lsb >> 6);
            
            // Check if gas measurement was skipped (0x80 in MSB is a flag)
            let gas_skipped = gas_r_msb == 0x80;
            
            defmt::debug!("Raw: T={} H={} P={} G={} (gas_msb=0x{:02X} gas_lsb=0x{:02X} skipped={})", 
                temp_raw, hum_raw, press_raw, gas_raw, gas_r_msb, gas_r_lsb, gas_skipped);

            // Validation
            let temp_valid = temp_raw > 10000 && temp_raw < 60000 && temp_raw != 32768 && temp_raw != 0 && temp_raw != 65535;
            let hum_valid = hum_raw > 0 && hum_raw < 65535 && hum_raw != 32768 && hum_raw != 65535;
            let press_valid = press_raw > 3000 && press_raw < 50000 && press_raw != 32768 && press_raw != 0 && press_raw != 65535;

            if temp_valid && press_valid {
                // Gas resistance handling
                // 0x3FF (1023) means invalid reading
                // gas_r_msb = 0x80 indicates measurement was skipped
                if gas_skipped {
                    // Measurement was skipped - don't update gas_resistance, keep previous value
                    warn!("⚠ Gas measurement skipped (gas_msb=0x80)");
                    // gas_resistance remains unchanged
                } else if gas_raw == 0x3FF || gas_raw == 0 {
                    gas_resistance = 0; // Invalid reading
                } else {
                    // Valid gas reading - store raw value for now
                    // TODO: Convert to actual ohms using calibration coefficients
                    // Reference shows values in kΩ (kilohms), typically 10-500 kΩ for clean air
                    gas_resistance = gas_raw;
                }

                // For now, use simplified formulas until calibration is fixed
                // The calibration coefficient parsing needs to be verified for BME688
                // Calculate temperature in tenths of a degree for one decimal place display
                temp_tenths = ((temp_raw as i32 * 10) / 1290) as i16;
                if hum_valid {
                    // Calculate humidity in tenths of a percent for one decimal place display
                    hum_tenths = ((hum_raw as i32 * 10) / 285) as i16;
                    if hum_tenths > 1000 { hum_tenths = 1000; } // Max 100.0%
                    if hum_tenths < 0 { hum_tenths = 0; }
                }
                // Pressure conversion: raw value to Pa
                // Calibrated for Brisbane: official pressure ~1016 hPa = 101600 Pa
                // For raw ~21412, target is 101600 Pa
                // Multiplier: 101600 / 21412 ≈ 4.744
                // Using integer math: (press_raw * 4744) / 1000
                press_int = ((press_raw as u32 * 4744) / 1000) as u32;

                // Format temperature and humidity for logging (tenths to decimal)
                let temp_whole = temp_tenths / 10;
                let temp_frac = (temp_tenths % 10).abs();
                let hum_whole = hum_tenths / 10;
                let hum_frac = (hum_tenths % 10).abs();
                info!("✓ BME680: {}.{}°C, {}.{}% RH, {} Pa, Gas: {}", temp_whole, temp_frac, hum_whole, hum_frac, press_int, gas_resistance);

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

                        // Draw sensor data - compact format
                        // Line 1: Temperature (one decimal place) and Humidity (one decimal place)
                        let temp_whole = temp_tenths / 10;
                        let temp_frac = (temp_tenths % 10).abs();
                        let hum_whole = hum_tenths / 10;
                        let hum_frac = (hum_tenths % 10).abs();
                        let mut temp_hum_buf = heapless::String::<32>::new();
                        if core::fmt::write(&mut temp_hum_buf, format_args!("T:{}.{}C H:{}.{}%", temp_whole, temp_frac, hum_whole, hum_frac)).is_ok() {
                            if Text::new(&temp_hum_buf, Point::new(2, 20), text_style).draw(&mut display).is_err() {
                                warn!("Failed to draw temp/hum");
                            }
                        }

                        // Line 2: Pressure
                        let press_hpa = press_int / 100;
                        let mut press_buf = heapless::String::<32>::new();
                        if core::fmt::write(&mut press_buf, format_args!("P:{}hPa", press_hpa)).is_ok() {
                            if Text::new(&press_buf, Point::new(2, 32), text_style).draw(&mut display).is_err() {
                                warn!("Failed to draw pressure");
                            }
                        }

                        // Line 3: Gas (if available)
                        let mut gas_buf = heapless::String::<32>::new();
                        if gas_resistance > 0 && gas_resistance != 0x3FF {
                            let _ = core::fmt::write(&mut gas_buf, format_args!("G:{}", gas_resistance));
                        } else {
                            let _ = core::fmt::write(&mut gas_buf, format_args!("G:--"));
                        }
                        if Text::new(&gas_buf, Point::new(2, 44), text_style).draw(&mut display).is_err() {
                            warn!("Failed to draw gas");
                        }

                        // Line 4: LoRaWAN status with SNR and RSSI
                        let mut lora_buf = heapless::String::<48>::new();
                        if join_successful {
                            let _ = core::fmt::write(&mut lora_buf, format_args!("L:J S:{} R:{} T:{}", last_snr, last_rssi, tx_count));
                        } else {
                            let _ = core::fmt::write(&mut lora_buf, format_args!("L:Joining..."));
                        }
                        if Text::new(&lora_buf, Point::new(2, 56), text_style).draw(&mut display).is_err() {
                            warn!("Failed to draw LoRaWAN status");
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

        // LoRaWAN operations
        if !join_successful {
            // Attempt OTAA join (retry every 5 seconds like working example)
            info!("Attempting LoRaWAN OTAA join...");
            let join_mode = JoinMode::OTAA {
                deveui: DevEui::from(DEV_EUI),
                appeui: AppEui::from(APP_EUI),
                appkey: AppKey::from(APP_KEY),
            };
            
            match device.join(&join_mode).await {
                Ok(JoinResponse::JoinSuccess) => {
                    info!("✓ LoRaWAN join successful!");
                    join_successful = true;
                    // Wait a bit after join to let I2C stabilize
                    Timer::after_secs(5).await;
                }
                Ok(JoinResponse::NoJoinAccept) => {
                    warn!("✗ LoRaWAN join failed: No join accept received. Retrying in 5 seconds...");
                    Timer::after_secs(5).await;
                }
                Err(_e) => {
                    warn!("✗ LoRaWAN join error. Retrying in 5 seconds...");
                    Timer::after_secs(5).await;
                }
            }
        } else {
            // Send sensor data every 15 seconds (7-8 loop iterations at 2s each)
            last_data_send += 1;
            if last_data_send >= 8 {
                info!("Sending sensor data via LoRaWAN...");
                
                // Prepare sensor data payload (4 bytes: temp, hum, press_msb, press_lsb)
                let mut payload = [0u8; 4];
                payload[0] = ((temp_tenths / 10) + 40) as u8; // Temp: -40 to +85°C, offset by 40 (use whole degrees)
                payload[1] = (hum_tenths / 10) as u8; // Humidity: 0-100% (use whole percent)
                payload[2] = ((press_int / 100) >> 8) as u8; // Pressure MSB (hPa)
                payload[3] = ((press_int / 100) & 0xFF) as u8; // Pressure LSB (hPa)
                
                match device.send(&payload, 1, false).await {
                    Ok(response) => {
                        match response {
                            SendResponse::DownlinkReceived(fcnt) => {
                                info!("✓ Data sent, downlink received (FCnt: {})", fcnt);
                                tx_count += 1;
                                // Update SNR and RSSI from device after downlink
                                last_snr = device.last_snr();
                                last_rssi = device.last_rssi();
                                info!("  SNR: {}, RSSI: {}", last_snr, last_rssi);
                            }
                            SendResponse::NoAck => {
                                info!("✓ Data sent (no ACK)");
                                tx_count += 1;
                                // Update SNR and RSSI even if no ACK (RX window might have completed)
                                last_snr = device.last_snr();
                                last_rssi = device.last_rssi();
                            }
                            SendResponse::RxComplete => {
                                info!("✓ Data sent, RX complete");
                                tx_count += 1;
                                // Update SNR and RSSI after RX complete
                                last_snr = device.last_snr();
                                last_rssi = device.last_rssi();
                            }
                            SendResponse::SessionExpired => {
                                warn!("✗ Session expired, need to rejoin");
                                join_successful = false;
                            }
                        }
                        // Note: These might be on the radio, not the device
                        // For now, try to access them if available
                        // TODO: Check if we need to access radio directly via device.radio() or similar
                        last_data_send = 0;
                        // Wait a bit after transmission to let I2C stabilize
                        Timer::after_secs(5).await;
                    }
                    Err(_e) => {
                        warn!("✗ Failed to send data");
                        Timer::after_secs(5).await;
                    }
                }
            }
        }

        Timer::after_secs(2).await;
    }
}
