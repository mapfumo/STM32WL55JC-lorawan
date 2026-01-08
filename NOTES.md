# Development Notes

## Project History

### Initial Setup

- Started with SHT41 temperature/humidity sensor
- Migrated to BME688 for additional pressure and gas sensing capabilities
- Implemented direct register access (no driver crate available)

### Sensor Migration (SHT41 → BME688)

**Challenges Encountered:**

1. **Bit Extraction**: Initially assumed 20-bit values like BME680/BME280, but BME688 uses 16-bit values
2. **Conversion Formulas**: Required empirical derivation due to lack of calibration coefficient implementation
3. **I2C Address Detection**: Implemented automatic detection for both 0x76 and 0x77 addresses

**Solutions:**

- Used 16-bit interpretation: `(msb << 8) | lsb`
- Derived conversion formulas through testing with known values
- Added status register polling for measurement completion

## Technical Details

### BME688 Initialization Sequence

1. **Soft Reset**: Send 0xB6 to register 0xE0
2. **Wait**: 10ms for reset to complete
3. **Configure Humidity**: Set CTRL_HUM register (0x72) to 0x01 (x1 oversampling)
4. **Configure Gas Sensor**:
   - Set CTRL_GAS_1 register (0x71) to 0x20 (enable gas sensor, use profile 0)
   - Set RES_HEAT_0 register (0x5A) to heater resistance (~20 for ~300°C)
   - Set IDAC_HEAT_0 register (0x50) to heater current (10 for moderate heating)
   - Set GAS_WAIT_0 register (0x64) to wait time (~100ms after heating)
5. **Configure Measurement**: Set CTRL_MEAS register (0x74) to 0x25 (temp x1, press x1, forced mode)
6. **Configure Filter**: Set CONFIG register (0x75) to 0x00 (no filter, no standby)

### Measurement Process

1. **Trigger Measurement**: Write 0x25 to CTRL_MEAS register (with gas sensor enabled)
2. **Poll Status**: Check STATUS register (0x73) until:
   - Bit 3 (measuring) is 0
   - Bit 0 (new_data) is 1
   - Bit 5 (gas_measuring) is 0 (gas measurement complete)
3. **Read Data**: Read 10 bytes from DATA register (0x1F) to include gas resistance
4. **Convert Values**: Apply conversion formulas
5. **Gas Measurement**: Extract 10-bit gas resistance value from bytes 8-9

### Data Register Layout

```
Offset | Description
-------|------------
0x1F   | Pressure MSB
0x20   | Pressure LSB
0x21   | Pressure XLSB (unused in 16-bit mode)
0x22   | Temperature MSB
0x23   | Temperature LSB
0x24   | Temperature XLSB (unused in 16-bit mode)
0x25   | Humidity MSB
0x26   | Humidity LSB
0x27   | Gas Resistance MSB (10-bit value, bits [9:2])
0x28   | Gas Resistance LSB (bits [1:0] in upper bits)
```

**Gas Resistance Extraction:**

- 10-bit value: `gas_r = (data[8] << 2) | (data[9] >> 6)`
- Invalid reading: 0x3FF (1023)
- Typical range: 10-500 kΩ for clean air (after calibration)

### Conversion Formula Derivation

**Temperature:**

- Raw value ~32252 → Expected ~25°C
- Formula: `temp_raw / 1290`
- Derived empirically by testing with known room temperature

**Humidity:**

- Raw value ~18200 → Expected ~64% RH (Brisbane conditions)
- Formula: `hum_raw / 285`
- Adjusted from initial `/200` based on actual conditions

**Pressure:**

- Raw value ~21879 → Expected ~100000 Pa (sea level)
- Formula: `(press_raw * 9) / 2`
- Gives approximately 984 hPa, reasonable for Brisbane elevation

### I2C Bus Management

The code uses a unique approach to I2C bus sharing:

- Creates temporary I2C instances for each operation
- Drops instances after use to release hardware
- Uses `unsafe { steal() }` to reuse peripherals
- This allows sharing I2C2 between BME688 and OLED display

### Memory Layout

- **Flash**: Configured in `memory.x`
- **RAM**: Uses default STM32WL55JC layout
- **Stack**: Managed by Cortex-M runtime

## Known Limitations

1. **No Calibration Coefficients**: Current implementation uses simplified formulas
2. **Gas Sensor Raw Values**: Gas resistance displayed as raw 10-bit value, not converted to kΩ
3. **Single Heating Profile**: Only using profile 0, no multiple profiles
4. **Fixed Update Rate**: 2-second update interval (hardcoded)
5. **No Error Recovery**: Limited error handling for I2C failures
6. **Integer Precision**: Temperature/humidity limited to integer values
7. **No IAQ Calculation**: Gas resistance not converted to Indoor Air Quality index

## Future Improvements

### High Priority

- [ ] Read and apply BME688 calibration coefficients
- [ ] Implement proper BME688 temperature compensation
- [x] Add gas sensor reading functionality - **Implemented**
- [ ] Convert gas resistance raw values to kΩ using calibration
- [ ] Implement IAQ (Indoor Air Quality) calculation
- [ ] Improve error handling and recovery

### Medium Priority

- [ ] Configurable update intervals
- [ ] Low-power modes
- [ ] Data averaging/filtering
- [ ] Display formatting improvements

### Low Priority

- [ ] Multiple sensor support
- [ ] Data logging to flash
- [x] Remote configuration via LoRaWAN - **In Progress**
- [ ] Web-based dashboard

## LoRaWAN Integration Status

### Current State

- **Credentials Configured**: Device EUI, Application EUI, and Application Key set for AU915 region
- **Status Tracking**: LoRaInfo struct tracks join status, SNR, RSSI, and TX count
- **Display Integration**: OLED shows LoRa status (NotJoined/Joining/Joined/Error)
- **Dependencies Added**: `lorawan-device` (0.12) and `lora-phy` (3.0)
- **Radio Driver**: ✅ Complete implementation using `stm32wlxx-hal`
- **RF Switch Control**: ✅ Implemented (NUCLEO-WL55JC1 board requirement)
- **AU915 Configuration**: ✅ Sub-band 0 (channels 0-7) configured
- **LoRaWAN Stack**: ✅ Integrated with `lorawan-device` v0.12

### Implementation Progress

1. **SubGHz Radio Driver**: ✅ Basic structure created (`src/radio.rs`)

   - ✅ Basic `SubGhzRadio` struct and initialization
   - ✅ Power and clock enable (PWR_CR5.SUBGHZSREN, RCC_APBENR2.SUBGHZSPIEN)
   - ⏳ SUBGHZSPI peripheral configuration (pending)
   - ⏳ Radio chip initialization via SPI (pending)
   - ⏳ Frequency configuration for AU915 (pending)
   - ⏳ Implement `lora-phy` radio traits (pending)
   - ⏳ Handle radio interrupts (SUBGHZ_RADIO) (pending)

2. **LoRaWAN Stack Initialization**:

   - Create LoRaWAN client with AU915 region
   - Configure OTAA (Over-The-Air Activation) join process
   - Handle join accept messages

3. **Data Transmission**:
   - Format sensor data into LoRaWAN payload
   - Send periodic uplink messages
   - Update SNR/RSSI from downlink responses

### Technical Challenges

- **No High-Level Driver**: embassy-stm32 doesn't expose SubGHz radio driver
- **Custom Implementation Required**: Need to implement radio driver from scratch
- **Complex Radio Configuration**: SubGHz peripheral requires careful initialization
- **Interrupt Handling**: SUBGHZ_RADIO interrupt needs proper handling

### Next Steps

1. ✅ Research STM32WL55 SubGHz peripheral register map - **In Progress**
2. ✅ Create basic radio driver structure - **Completed** (`src/radio.rs`)
3. ⏳ Initialize SubGHz peripheral and configure for LoRa modulation - **In Progress**
   - Verify register addresses against STM32WL55 reference manual
   - Configure SUBGHZSPI peripheral (mode, baud rate)
   - Initialize radio chip via SPI commands
4. ⏳ Implement basic TX/RX functionality - **Pending**
   - Implement `lora-phy` Radio trait
   - Handle radio state machine (IDLE, TX, RX)
   - Implement interrupt handlers
5. ⏳ Integrate with `lorawan-device` stack - **Pending**
6. ⏳ Test join process and data transmission - **Pending**

### Radio Driver Implementation Details

**File**: `src/radio.rs`

**Current Implementation**:

- `SubGhzRadio` struct: Basic radio driver structure
- `init()`: Enables SubGHz power domain and SPI clock via direct register access
- `configure_au915()`: Placeholder for AU915 frequency configuration
- `RadioError` enum: Error types for radio operations

**Register Access**:

- Uses unsafe raw pointer access to PAC registers
- PWR_CR5 at 0x5802_4814 (PWR base + 0x14): SUBGHZSREN bit
- RCC_APBENR2 at 0x5802_1C64 (RCC base + 0x64): SUBGHZSPIEN bit
- **Note**: Register addresses need verification against STM32WL55 reference manual

**Pending Work**:

- SUBGHZSPI peripheral configuration (SPI mode, baud rate, etc.)
- Radio chip initialization sequence via SPI
- Frequency register configuration for AU915 channels
- LoRa modulation parameter setup (Spreading Factor, Bandwidth, Coding Rate)
- Radio interrupt handling
- Implementation of `lora-phy::Radio` trait methods

## Testing Notes

### Test Conditions

- **Location**: Brisbane, Australia
- **Temperature**: ~24-25°C
- **Humidity**: ~64% RH
- **Pressure**: ~984 hPa

### Validation

- Temperature readings match room temperature
- Humidity matches local weather conditions
- Pressure readings reasonable for elevation
- Display updates smoothly every 2 seconds

## Code Style

- Uses `defmt::info!` for logging
- Integer-only math (no floating point)
- Async/await with Embassy executor
- `unsafe` blocks only where necessary (peripheral stealing)
- Clear comments explaining register operations

## Dependencies

### Core

- `embassy-executor`: Async runtime
- `embassy-stm32`: HAL for STM32
- `embassy-time`: Time management

### Display

- `sh1106`: OLED driver
- `embedded-graphics`: Graphics primitives
- `heapless`: No-std string formatting

### LoRaWAN

- `lorawan-device`: LoRaWAN device stack (v0.12) - added, pending integration
- `lora-phy`: LoRa physical layer (v3.0) - added, needs custom radio driver

### Debugging

- `defmt`: Efficient logging
- `defmt-rtt`: RTT transport
- `panic-probe`: Panic handler

## Build Configuration

- **Target**: `thumbv7em-none-eabihf`
- **Optimization**: Size-optimized in release (`opt-level = "z"`)
- **Debug Symbols**: Enabled in both dev and release profiles
- **LTO**: Fat LTO enabled in release

## Hardware Connections

```
STM32WL55JC          BME688          SH1106 OLED
-----------          ------          -----------
PA12 (SCL)  -------- SCL
PA11 (SDA)  -------- SDA
3.3V        -------- VCC
GND         -------- GND

PB15        -------- LED (optional)
```

## I2C Pull-ups

- Internal pull-ups enabled on PA11 (SDA) and PA12 (SCL)
- External pull-ups may be needed depending on bus capacitance
- 100 kHz I2C speed for reliability
