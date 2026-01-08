# STM32WL55 LoRaWAN Sensor Node - Complete Documentation

## Project Overview

This project implements a complete LoRaWAN sensor node on the STM32WL55JC microcontroller. It reads environmental data from a BME688 sensor, displays it on an SH1106 OLED display, and transmits the data via LoRaWAN to a gateway.

**Status**: Fully functional - sensor readings, display, and LoRaWAN transmission working.

## Architecture

### Hardware Components

1. **STM32WL55JC Microcontroller**
   - Cortex-M4 core (M4 used, M0+ available)
   - Integrated SubGHz radio (SX126x compatible)
   - Dual-core architecture with security features

2. **BME688 Environmental Sensor**
   - Temperature, humidity, pressure, and gas resistance
   - I2C interface (address 0x76 or 0x77)
   - Metal-oxide gas sensor with configurable heating

3. **SH1106 OLED Display**
   - 128x64 pixel monochrome display
   - I2C interface (address 0x3C)
   - Used for real-time sensor data display

4. **NUCLEO-WL55JC1 Board**
   - RF switch for antenna routing (PC3, PC4, PC5)
   - LED indicator (PB15)
   - USB connection for programming/debugging

### Software Stack

```
┌─────────────────────────────────────┐
│         Application Layer           │
│  (main.rs - sensor, display, loop)  │
└─────────────────────────────────────┘
              │
              ├─── LoRaWAN Device Stack
              │    (lorawan-device v0.12)
              │
              ├─── LoRa Physical Layer
              │    (lora-phy v3.0)
              │
              ├─── Interface Variant
              │    (iv.rs - RF switch control)
              │
              ├─── Embassy Framework
              │    (embassy-stm32 v0.2.0)
              │
              └─── Hardware Abstraction
                   (I2C, SPI, GPIO, RNG)
```

## Dependencies and Patches

### Core Dependencies

- **embassy-stm32**: `0.2.0` - STM32 HAL with async support
- **embassy-time**: `0.4.0` - Time and timer abstractions
- **embassy-executor**: `0.7.0` - Async executor
- **embassy-sync**: `0.6` - Synchronization primitives

### LoRaWAN Stack (Patched)

The project uses **patched versions** of `lora-phy` and `lorawan-device` to resolve dependency conflicts:

1. **lorawan-device-patched** (`/home/tony/dev/lorawan-device-patched`)
   - Based on `lorawan-device v0.12.2`
   - Modified to use `embassy-time v0.4.0` instead of `v0.3.0`
   - Patched to expose SNR/RSSI values via `last_snr()` and `last_rssi()` methods

2. **lora-phy-patched** (`/home/tony/dev/lora-phy-patched`)
   - Based on `lora-phy v3.0.1`
   - Modified to use local `lorawan-device-patched` instead of crates.io version

**Why Patched?**
- `embassy-stm32 v0.2.0` requires `embassy-time v0.4.0`
- `lorawan-device v0.12` requires `embassy-time v0.3.0`
- Direct patching via `[patch.crates-io]` doesn't work (must point to different sources)
- Solution: Copy and modify the crates locally

### Display and Graphics

- **sh1106**: `0.5` - OLED display driver
- **embedded-graphics**: `0.8` - Graphics primitives
- **heapless**: `0.8` - No-std string formatting

### Other Dependencies

- **defmt**: `0.3` - Efficient logging framework
- **defmt-rtt**: `0.4` - RTT transport for defmt
- **panic-probe**: `0.3` - Panic handler with defmt support
- **cortex-m**: `0.7.7` - Cortex-M runtime support

## Configuration

### Clock Configuration

```rust
// 32MHz HSE (external crystal)
config.rcc.hse = Some(Hse {
    freq: Hertz(32_000_000),
    mode: HseMode::Bypass,
    prescaler: HsePrescaler::DIV1,
});

// PLL configuration for 48MHz system clock
config.rcc.sys = Sysclk::PLL1_R;
config.rcc.pll = Some(Pll {
    source: PllSource::HSE,
    prediv: PllPreDiv::DIV2,
    mul: PllMul::MUL6,
    divp: None,
    divq: Some(PllQDiv::DIV2),
    divr: Some(PllRDiv::DIV2),
});
```

**Result**: 48MHz system clock (required for SubGHz radio)

### I2C Configuration

```rust
let mut i2c_config = I2cConfig::default();
i2c_config.sda_pullup = true;
i2c_config.scl_pullup = true;
// Speed: 100 kHz (Hertz(100_000))
```

**Pins**:
- SCL: PA12
- SDA: PA11
- Peripheral: I2C2

### LoRaWAN Configuration

**Region**: AU915 (Australia 915 MHz band)
- Subband: 1 (channels 8-15)
- Max TX Power: 14 dBm

**Credentials** (OTAA - Over-The-Air Activation):
```rust
DEV_EUI:  AC1F09FFFE1BCE23
APP_EUI:  B130A864C5295356
APP_KEY:  B726739B78EC4B9E9234E5D35EA9681B
```

**Join Mode**: OTAA (device joins network on startup)

### BME688 Sensor Configuration

```rust
// Humidity oversampling: x1
CTRL_HUM: 0x01

// Gas sensor enabled, profile 0
CTRL_GAS_1: 0x20

// Heater configuration for profile 0
RES_HEAT_0: 20        // ~300°C heater temperature
IDAC_HEAT_0: 10       // Moderate heating current
GAS_WAIT_0: 0x65      // Wait time after heating

// Standby time: 0ms (forced mode)
CONFIG: 0x00
```

**Measurement Mode**: Forced mode (triggered on demand)
- Trigger: Write `CTRL_MEAS` with forced mode bit
- Wait: 500ms for measurement completion
- Read: 10 bytes from `REG_DATA` (0x1F)

## Code Structure

### Main Application Flow

```
1. Initialize peripherals
   ├── Clock configuration (48MHz)
   ├── I2C2 for sensor/display
   ├── LED (PB15)
   └── RNG for LoRaWAN crypto

2. Detect and initialize BME688
   ├── Scan I2C for sensor (0x76 or 0x77)
   ├── Read chip ID (0x61)
   └── Configure sensor registers

3. Initialize LoRaWAN radio
   ├── Configure SubGHz SPI
   ├── Initialize SX126x radio
   ├── Set up RF switch (PC3, PC4, PC5)
   └── Configure AU915 region

4. Join LoRaWAN network (OTAA)
   ├── Send join request
   ├── Wait for join accept (RX1/RX2 windows)
   └── Retry on failure (5 second intervals)

5. Main sensor loop
   ├── Read BME688 sensor
   ├── Update OLED display
   ├── Send data via LoRaWAN (every 60 seconds)
   └── Handle errors and retries
```

### Key Modules

#### `src/main.rs`
- Main application entry point
- Sensor reading and processing
- Display management
- LoRaWAN transmission
- Error handling and recovery

#### `src/iv.rs`
- `InterfaceVariant` trait implementation
- RF switch control (TX/RX/OFF)
- Radio interrupt handling
- GPIO management for RF switch

### Sensor Reading Process

1. **Trigger Measurement**
   ```rust
   // Set forced mode
   i2c.blocking_write(addr, &[CTRL_MEAS, 0x25])
   Timer::after_millis(500).await; // Wait for measurement
   ```

2. **Read Data**
   ```rust
   // Read 10 bytes from REG_DATA (0x1F)
   let data = [press_msb, press_lsb, press_xlsb,
               temp_msb, temp_lsb, temp_xlsb,
               hum_msb, hum_lsb,
               gas_r_msb, gas_r_lsb];
   ```

3. **Extract Raw Values**
   ```rust
   press_raw = (data[0] << 8) | data[1];
   temp_raw = (data[3] << 8) | data[4];
   hum_raw = (data[6] << 8) | data[7];
   gas_raw = (data[8] << 2) | (data[9] >> 6); // 10-bit value
   ```

4. **Validate and Convert**
   ```rust
   // Reject invalid values (0x0000, 0x8000, 0xFFFF)
   if temp_valid && press_valid {
       temp_int = temp_raw / 1290;
       press_int = (press_raw * 9) / 2;
       if hum_valid {
           hum_int = hum_raw / 285;
       }
   }
   ```

### Display Update Process

1. **Create Fresh I2C Instance**
   - Drop sensor I2C
   - Wait 200ms for bus to settle
   - Create new I2C instance for display

2. **Initialize Display**
   ```rust
   let display = Builder::new()
       .with_size(Display128x64)
       .connect_i2c(i2c)
       .into();
   display.init()?;
   ```

3. **Draw Content**
   - Clear display buffer
   - Draw title: "Node1"
   - Draw sensor data: "T:25C H:64%"
   - Draw pressure: "P:984 hPa"
   - Draw gas: "G:512"
   - Draw LoRaWAN status: "L:J S:4 R:-25 T:1"

4. **Flush to Hardware**
   ```rust
   display.flush()?; // With retry logic (5 attempts)
   ```

**Why Recreate Display Each Time?**
- Ensures fresh I2C instance after radio operations
- Prevents I2C bus corruption from radio interference
- Matches working solution pattern

### LoRaWAN Transmission

**Transmission Interval**: Every 60 seconds
- Loop runs every ~2 seconds
- Counter increments each iteration
- When counter >= 30, send data (30 × 2s = 60s)

**Payload Format** (4 bytes):
```rust
payload[0] = (temp_int + 40) as u8;  // Temperature offset by 40°C
payload[1] = hum_int as u8;          // Humidity (0-100%)
payload[2] = (press_hpa >> 8) as u8; // Pressure MSB (hPa)
payload[3] = (press_hpa & 0xFF) as u8; // Pressure LSB (hPa)
```

**Transmission Process**:
1. Check if 60 seconds elapsed
2. Prepare payload from sensor data
3. Call `device.send(&payload, 1, false).await`
4. Handle response:
   - `DownlinkReceived`: ACK received
   - `NoAck`: No ACK (still successful)
   - `RxComplete`: RX window completed
   - `SessionExpired`: Need to rejoin
5. Increment TX counter
6. Reset timer

## Display Format

### OLED Layout (128x64 pixels)

```
┌────────────────────────────────────┐
│ Node1                              │  Line 1 (Y=8)
│                                    │
│ T:25C  H:64%                       │  Line 2 (Y=20)
│                                    │
│ P:984 hPa                          │  Line 3 (Y=32)
│                                    │
│ G:512                              │  Line 4 (Y=44)
│                                    │
│ L:J S:4 R:-25 T:1                  │  Line 5 (Y=56)
└────────────────────────────────────┘
```

### Status Line Format

**When Joined**:
- `L:J` - LoRaWAN Joined
- `S:XX` - SNR (Signal-to-Noise Ratio), or `--` if no RX yet
- `R:XXX` - RSSI (Received Signal Strength), or `---` if no RX yet
- `T:XX` - TX count (number of successful transmissions, max 99)

**When Not Joined**:
- `L:NotJoined`

### Font and Styling

- **Font**: `FONT_6X10` (6px wide, 10px tall)
- **Color**: `BinaryColor::On` (white on black)
- **Positioning**: Left-aligned, 2px margin

## Sensor Data Processing

### Conversion Formulas

**Temperature**:
```rust
temp_int = temp_raw / 1290  // °C (empirically derived)
```
- Range: -40°C to +85°C
- Raw range: ~15000 to ~50000

**Humidity**:
```rust
hum_int = hum_raw / 285  // % RH (empirically derived)
if hum_int > 100 { hum_int = 100; }
if hum_int < 0 { hum_int = 0; }
```
- Range: 0% to 100% RH
- Raw range: ~0 to ~28500

**Pressure**:
```rust
press_int = (press_raw * 9) / 2  // Pa
press_hpa = press_int / 100      // hPa
```
- Range: 300 to 1100 hPa
- Raw range: ~5000 to ~40000

**Gas Resistance**:
```rust
gas_resistance = gas_raw  // 10-bit raw value (0-1023)
```
- Displayed as-is (no conversion)
- Invalid value: 0x3FF (1023)
- Typical clean air: 10-500 kΩ (after calibration)

### Validation

**Rejected Values**:
- `0x0000` - Zero (invalid)
- `0x8000` (32768) - "Not ready" indicator
- `0xFFFF` (65535) - Maximum (invalid)

**Partial Readings**:
- If temperature and pressure are valid, accept them
- If humidity is invalid, keep previous value
- This allows display updates even if humidity isn't ready

### Error Handling

**Sensor Failures**:
- Count consecutive failures
- After 5 failures, reinitialize sensor
- Skip first measurement after reinit (stabilization)

**Display Failures**:
- Retry flush up to 5 times with 100ms delays
- If all retries fail, skip update (will retry next iteration)
- Display is recreated each loop, so failures don't persist

**LoRaWAN Failures**:
- Join retries every 5 seconds
- Session expiration triggers rejoin
- TX counter tracks successful transmissions

## I2C Bus Management

### Shared Bus Architecture

Both BME688 sensor and SH1106 display share I2C2:
- **Sensor**: Address 0x76 or 0x77
- **Display**: Address 0x3C

### Bus Sharing Strategy

1. **Create I2C instance for sensor**
   - Read sensor data
   - Drop I2C instance

2. **Wait for bus to settle** (200ms)
   - Ensures I2C peripheral is released
   - Prevents bus corruption

3. **Create I2C instance for display**
   - Initialize display
   - Draw content
   - Flush to hardware
   - Drop I2C instance

**Why This Works**:
- Sensor and display are never accessed simultaneously
- Each operation gets a fresh I2C instance
- Dropping instances releases the hardware properly

### Radio Interference Mitigation

**Problem**: LoRa radio operations can corrupt I2C bus state

**Solutions**:
1. **Long delays after join** (5 seconds)
2. **I2C bus reset** (create/drop instances 3 times)
3. **Delays before display** (200ms after sensor read)
4. **Display retry logic** (5 attempts with delays)
5. **Fresh I2C instances** (recreate each loop)

## RF Switch Control

### GPIO Pins

- **PC3**: Control line 3
- **PC4**: Control line 1
- **PC5**: Control line 2

### Switch States

**TX Mode**:
```rust
ctrl1.set_high();  // PC4
ctrl2.set_low();   // PC5
ctrl3.set_high();  // PC3
```

**RX Mode**:
```rust
ctrl1.set_low();   // PC4
ctrl2.set_low();   // PC5
ctrl3.set_high();  // PC3
```

**OFF Mode**:
```rust
ctrl1.set_low();   // PC4
ctrl2.set_low();   // PC5
ctrl3.set_low();   // PC3
```

### Implementation

See `src/iv.rs` for `Stm32wlInterfaceVariant` implementation:
- `enable_rf_switch_tx()` - Called before TX
- `enable_rf_switch_rx()` - Called before RX
- `disable_rf_switch()` - Called after TX/RX

## LoRaWAN Integration Details

### Radio Initialization

```rust
// SubGHz SPI configuration
let spi = Spi::new_subghz(p.SUBGHZSPI, p.DMA1_CH1, p.DMA1_CH2);
let spi = SubghzSpiDevice(spi);

// Radio configuration
let config = sx126x::Config {
    chip: Stm32wl { use_high_power_pa: true },
    tcxo_ctrl: Some(TcxoCtrlVoltage::Ctrl1V7),
    use_dcdc: true,
    rx_boost: false,
};

// Interface variant (RF switch control)
let iv = Stm32wlInterfaceVariant::new(
    Irqs, true, Some(ctrl1), Some(ctrl2), Some(ctrl3)
)?;

// Initialize LoRa
let lora = LoRa::new(Sx126x::new(spi, iv, config), true, Delay).await?;
let radio: LorawanRadio<_, _, MAX_TX_POWER> = lora.into();
```

### Region Configuration

```rust
let mut au915 = AU915::new();
au915.set_join_bias(Subband::_1);  // Channels 8-15
let region: region::Configuration = au915.into();
```

### Device Creation

```rust
let mut device: Device<_, DefaultFactory, _, _> = Device::new(
    region,
    radio,
    EmbassyTimer::new(),
    Rng::new(p.RNG, Irqs)
);
```

### Join Process

```rust
let join_mode = JoinMode::OTAA {
    deveui: DevEui::from(DEV_EUI),
    appeui: AppEui::from(APP_EUI),
    appkey: AppKey::from(APP_KEY),
};

loop {
    match device.join(&join_mode).await {
        Ok(JoinResponse::JoinSuccess) => break,
        Ok(JoinResponse::NoJoinAccept) => {
            // Retry after 5 seconds
        }
        Err(e) => {
            // Handle error
        }
    }
    Timer::after_secs(5).await;
}
```

### Data Transmission

```rust
match device.send(&payload, 1, false).await {
    Ok(SendResponse::DownlinkReceived(fcnt)) => {
        // ACK received
    }
    Ok(SendResponse::NoAck) => {
        // No ACK (still successful)
    }
    Ok(SendResponse::RxComplete) => {
        // RX window completed
    }
    Ok(SendResponse::SessionExpired) => {
        // Need to rejoin
        join_successful = false;
    }
}
```

## Known Issues and Workarounds

### 1. I2C Bus Corruption After Radio Operations

**Symptom**: Display flush fails after LoRa join/transmission

**Workaround**:
- Long delays after radio operations (5 seconds after join)
- I2C bus reset (create/drop instances multiple times)
- Fresh I2C instances for each operation
- Retry logic for display flush (5 attempts)

### 2. Humidity Reading Invalid After Radio Operations

**Symptom**: Humidity reads 32768 (0x8000) after LoRa join

**Workaround**:
- Increased measurement wait time (500ms instead of 300ms)
- Accept partial readings (temp + pressure even if humidity invalid)
- Keep previous humidity value if new reading is invalid

### 3. Display Shows Partial Data

**Symptom**: Only 2/3 of OLED displays correctly

**Workaround**:
- Multiple flush retries with delays
- Fresh display instance each loop
- Delays after init and clear operations

### 4. Dependency Version Conflicts

**Symptom**: `embassy-time` version conflict between `embassy-stm32` and `lorawan-device`

**Workaround**:
- Use patched versions of `lorawan-device` and `lora-phy`
- Patches modify `embassy-time` dependency to v0.4.0
- Patches stored in `/home/tony/dev/lorawan-device-patched` and `/home/tony/dev/lora-phy-patched`

## Build and Flash

### Prerequisites

```bash
# Install Rust toolchain
rustup target add thumbv7em-none-eabihf

# Install probe-rs
cargo install probe-rs --locked

# Disable security (required for SubGHz radio)
./disable_security.sh
```

### Build

```bash
# Development build
cargo build

# Release build (optimized for size)
cargo build --release
```

### Flash

```bash
# Flash and run (development)
cargo run

# Flash and run (release)
cargo run --release
```

### Monitor Output

```bash
# Using probe-rs RTT
probe-rs rtt --chip STM32WL55JCIx

# Or use defmt-print
cargo run 2>&1 | defmt-print
```

## File Structure

```
lora_1/
├── src/
│   ├── main.rs          # Main application
│   ├── iv.rs            # Interface variant (RF switch)
│   ├── main_old.rs       # Old implementation (backup)
│   └── main_working.rs   # Working reference (backup)
├── Cargo.toml            # Dependencies
├── Cargo.lock            # Locked dependencies
├── memory.x              # Memory layout
├── build.rs              # Build script
├── README.md             # Basic readme
├── PROJECT_DOCUMENTATION.md  # This file
├── NOTES.md              # Development notes
├── TROUBLESHOOTING.md    # Troubleshooting guide
├── SECURITY_SETUP.md     # Security configuration
├── LORAWAN_INTEGRATION.md # LoRaWAN details
└── disable_security.sh   # Security disable script
```

## Testing

### Manual Testing Checklist

- [x] BME688 sensor detection and initialization
- [x] Sensor readings (temperature, humidity, pressure, gas)
- [x] OLED display initialization
- [x] Display update with sensor data
- [x] LoRaWAN join (OTAA)
- [x] LoRaWAN data transmission
- [x] SNR/RSSI display
- [x] TX counter display
- [x] Error recovery (sensor failures, display failures)
- [x] I2C bus recovery after radio operations

### Expected Behavior

1. **On Startup**:
   - LED toggles
   - Sensor detected and initialized
   - LoRaWAN join attempt
   - Display shows "L:NotJoined" until join succeeds

2. **After Join**:
   - Display shows "L:J S:-- R:--- T:0"
   - Sensor readings displayed
   - TX counter increments every 60 seconds

3. **During Operation**:
   - Display updates every ~2 seconds with new sensor data
   - LoRaWAN transmission every 60 seconds
   - TX counter increments after each transmission
   - SNR/RSSI updates after each RX

## Future Enhancements

### Planned Features

- [ ] MQTT integration (next step)
- [ ] Gas sensor calibration coefficient reading
- [ ] IAQ (Indoor Air Quality) calculation
- [ ] Multiple gas sensor heating profiles
- [ ] Data logging to flash
- [ ] Low-power modes
- [ ] Battery monitoring
- [ ] Over-the-air firmware updates

### MQTT Integration (Next Step)

**Requirements**:
- LoRaWAN to MQTT bridge/gateway
- Topic structure design
- Payload format standardization
- QoS levels
- Retain flags
- Last will and testament

**Considerations**:
- Gateway must decode LoRaWAN payloads
- MQTT broker configuration
- Topic naming convention
- Message format (JSON vs binary)
- Timestamp handling
- Device identification

## References

### Documentation

- [STM32WL55 Reference Manual](https://www.st.com/resource/en/reference_manual/rm0453-stm32wl5x-advanced-armbased-32bit-mcus-stmicroelectronics.pdf)
- [BME688 Datasheet](https://www.bosch-sensortec.com/products/environmental-sensors/gas-sensors/bme688/)
- [LoRaWAN Specification](https://lora-alliance.org/lorawan-for-developers/)
- [Embassy Documentation](https://embassy.dev/)

### Related Projects

- `/home/tony/dev/4-month-plan/lora_known_working` - Working LoRaWAN solution
- `/home/tony/dev/4-month-plan/wk10-lorawan/firmware/node1-bme688` - Reference implementation

### Patched Dependencies

- `/home/tony/dev/lorawan-device-patched` - Patched lorawan-device
- `/home/tony/dev/lora-phy-patched` - Patched lora-phy

## License

[Add license information]

## Author

[Add author information]

## Version History

- **v0.1.0** (Current)
  - Initial working implementation
  - BME688 sensor integration
  - OLED display
  - LoRaWAN OTAA join
  - Data transmission every 60 seconds
  - SNR/RSSI display
  - TX counter
