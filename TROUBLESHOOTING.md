# Troubleshooting Guide

## Common Issues and Solutions

### Sensor Not Detected

**Symptoms:**

- I2C scan shows no device at 0x76 or 0x77
- "BME688 not found" messages
- Chip ID read fails

**Possible Causes:**

1. **Wiring Issues**

   - Check SDA/SCL connections (PA11/PA12)
   - Verify power connections (3.3V and GND)
   - Ensure I2C bus is not shorted

2. **I2C Address Mismatch**

   - BME688 can be at 0x76 (default) or 0x77
   - Check ADR jumper/pin on sensor board
   - Code automatically detects both addresses

3. **Pull-up Resistors**

   - I2C requires pull-up resistors (typically 4.7kΩ)
   - Internal pull-ups are enabled, but may be insufficient
   - Add external pull-ups if bus capacitance is high

4. **Power Issues**
   - Verify 3.3V power supply is stable
   - Check for voltage drops under load
   - Ensure adequate current capacity

**Solutions:**

```rust
// Check I2C scan output in logs
// Should see: "✓ Device at 0x76" or "✓ Device at 0x77"
// If not, verify hardware connections
```

### Incorrect Sensor Readings

**Symptoms:**

- Temperature shows unrealistic values (e.g., 5000°C, -166°C)
- Humidity > 100% or < 0%
- Pressure values way off

**Possible Causes:**

1. **Wrong Bit Extraction**

   - BME688 uses 16-bit values, not 20-bit
   - Check raw data bytes in debug output
   - Verify bit extraction matches sensor format

2. **Conversion Formula Issues**

   - Formulas are empirically derived
   - May need adjustment for your sensor unit
   - Calibration coefficients not applied

3. **Sensor Not Initialized**
   - Reset command may have failed
   - Configuration registers not set correctly
   - Measurement not completed before reading

**Solutions:**

**Check Raw Values:**

```rust
// Enable debug output to see raw values
info!("Raw values: press={}, temp={}, hum={}", press_raw, temp_raw, hum_raw);
```

**Verify Initialization:**

- Check that "BME688 initialized @ 0x76" appears in logs
- Verify chip ID reads as 0x61
- Ensure status register shows measurement complete

**Adjust Conversion Formulas:**
If readings are consistently off by a factor:

```rust
// Temperature adjustment example
// If reading 250°C but should be 25°C, divide by 10
temp_int = (temp_raw_i32 / 1290) as i16;  // Current formula

// Humidity adjustment example
// If reading 91% but should be 64%, adjust divisor
hum_int = (hum_raw_i32 / 285) as i16;  // Current formula
```

### OLED Display Not Working

**Symptoms:**

- Display shows nothing
- Display shows garbage/corrupted data
- "Failed to init OLED" error

**Possible Causes:**

1. **I2C Address Mismatch**

   - SH1106 typically at 0x3C
   - Verify address matches your display

2. **Display Initialization**

   - Display may need longer initialization time
   - Reset sequence may be required

3. **I2C Bus Conflicts**
   - BME688 and OLED sharing same bus
   - Bus not released properly between operations

**Solutions:**

**Check I2C Scan:**

```rust
// Should see: "✓ Device at 0x3C" in scan output
```

**Add Delay After Init:**

```rust
if display.init().is_ok() {
    Timer::after_millis(100).await;  // Add delay if needed
    display.clear();
    // ...
}
```

**Verify Display Type:**

- Confirm SH1106 (not SSD1306)
- Check display size matches (128x64)

### Compilation Errors

**Symptoms:**

- `cargo build` fails
- Missing target errors
- Linker errors

**Solutions:**

**Missing Target:**

```bash
rustup target add thumbv7em-none-eabihf
```

**Missing probe-rs:**

```bash
cargo install probe-rs --locked
```

**Memory Layout Issues:**

- Check `memory.x` file exists
- Verify memory sizes match STM32WL55JC specs
- Ensure build.rs includes memory.x

### Runtime Errors

**Symptoms:**

- Panic messages in logs
- Device resets unexpectedly
- Hangs during execution

**Possible Causes:**

1. **Stack Overflow**

   - Increase stack size in memory.x
   - Check for large local variables

2. **I2C Bus Lock**

   - Bus not released properly
   - Multiple simultaneous access attempts

3. **Timer Issues**
   - Timer not initialized correctly
   - Clock configuration problems

**Solutions:**

**Check Stack Size:**

```rust
// In memory.x
_STACK_SIZE = 0x2000;  // Increase if needed
```

**Add Error Handling:**

```rust
match i2c.blocking_write(addr, &data) {
    Ok(_) => info!("Success"),
    Err(e) => error!("I2C error: {:?}", e),
}
```

### Incorrect Humidity Readings

**Symptoms:**

- Humidity consistently too high/low
- Humidity > 100% or < 0%

**Solutions:**

**Adjust Conversion Formula:**

```rust
// Current: hum_int = (hum_raw_i32 / 285) as i16;
// If reading 91% but should be 64%:
// New divisor = (18200 / 64) ≈ 284
hum_int = (hum_raw_i32 / 284) as i16;

// Clamp to valid range
if hum_int > 100 { hum_int = 100; }
if hum_int < 0 { hum_int = 0; }
```

**Check Sensor Placement:**

- Avoid direct airflow
- Keep away from heat sources
- Allow sensor to stabilize (24+ hours recommended by Bosch)

### Pressure Readings Off

**Symptoms:**

- Pressure significantly different from local weather
- Pressure not changing with altitude

**Solutions:**

**Adjust Conversion Formula:**

```rust
// Current: press_int = ((press_raw * 9) / 2) as u32;
// If consistently off, adjust multiplier
// For example, if reading 984 hPa but should be 1013 hPa:
// New multiplier = (1013 * 2) / 21879 ≈ 0.0926
// Try: press_int = ((press_raw * 19) / 20) as u32;
```

**Sea Level Adjustment:**

- Pressure decreases with altitude (~1 hPa per 8m)
- Brisbane is ~30m above sea level
- Adjust formula based on your elevation

### Gas Sensor Issues

**Symptoms:**

- Gas resistance always shows 0 or invalid (--)
- Gas resistance stuck at same value (e.g., 512)
- Gas measurement takes too long or times out

**Possible Causes:**

1. **Gas Sensor Not Enabled**

   - Check initialization logs for "gas sensor enabled"
   - Verify CTRL_GAS_1 register is set correctly (0x20)

2. **Heating Profile Not Configured**

   - Gas sensor requires heating before measurement
   - Check RES_HEAT_0, IDAC_HEAT_0, and GAS_WAIT_0 registers
   - Verify heater resistance is set (typically 20 for ~300°C)

3. **Measurement Timeout**

   - Gas measurements take longer than temp/humidity/pressure
   - Current timeout is 400ms (20 attempts × 20ms)
   - May need to increase timeout for slower measurements

4. **Invalid Reading (0x3FF)**
   - Gas resistance value of 1023 (0x3FF) indicates invalid measurement
   - Check status register bit 5 (gas_measuring) is cleared
   - Verify gas sensor is properly initialized

**Solutions:**

**Check Gas Sensor Status:**

```rust
// Add debug output to check gas sensor status
let mut status = [0u8; 1];
if i2c.blocking_write_read(bme688_addr, &[BME688_REG_STATUS], &mut status).is_ok() {
    info!("Status: 0x{:02X}", status[0]);
    // Bit 5 = gas_measuring (should be 0 when done)
    // Bit 3 = measuring (should be 0 when done)
    // Bit 0 = new_data (should be 1 when ready)
}
```

**Increase Measurement Timeout:**

```rust
// If gas measurements timeout, increase attempts
let mut attempts = 0;
loop {
    Timer::after_millis(20).await;
    // ... check status ...
    attempts += 1;
    if attempts > 30 {  // Increased from 20 to 30
        break; // Timeout
    }
}
```

**Verify Gas Sensor Configuration:**

```rust
// Check gas sensor registers
let mut ctrl_gas1 = [0u8; 1];
if i2c.blocking_write_read(bme688_addr, &[BME688_REG_CTRL_GAS_1], &mut ctrl_gas1).is_ok() {
    info!("CTRL_GAS_1: 0x{:02X} (should be 0x20)", ctrl_gas1[0]);
    // Bit 5 should be 1 (gas sensor enabled)
}
```

**Gas Resistance Conversion:**

- Current implementation shows raw 10-bit value (0-1023)
- To convert to kΩ, need calibration coefficients from sensor
- Typical clean air: 10-500 kΩ (after calibration)
- Lower values indicate higher gas concentration

**Gas Sensor Heating:**

- Heater temperature controlled by RES_HEAT_0 register
- Formula: `res_heat = (desired_temp - 200) / 5`
- Example: For 300°C: (300 - 200) / 5 = 20
- Higher temperature = faster response but more power consumption

### I2C Bus Errors

**Symptoms:**

- "Failed to read BME688 data" errors
- "Failed to trigger BME688 measurement" errors
- Intermittent sensor readings

**Solutions:**

**Increase I2C Timeout:**

```rust
// In I2cConfig, if timeout available
i2c_config.timeout = Some(Duration::from_millis(100));
```

**Add Retry Logic:**

```rust
let mut retries = 3;
while retries > 0 {
    if i2c.blocking_write(addr, &data).is_ok() {
        break;
    }
    retries -= 1;
    Timer::after_millis(10).await;
}
```

**Check Bus Speed:**

```rust
// Reduce speed if having issues
Hertz(50_000),  // Instead of 100_000
```

### Debugging Tips

**Enable More Logging:**

```rust
// Add debug output for raw values
info!("Raw data: {:02X} {:02X} ...", data[0], data[1], ...);
info!("Raw values: press={}, temp={}, hum={}", press_raw, temp_raw, hum_raw);
```

**Check Status Register:**

```rust
let mut status = [0u8; 1];
if i2c.blocking_write_read(bme688_addr, &[BME688_REG_STATUS], &mut status).is_ok() {
    info!("Status: 0x{:02X}", status[0]);
    // Bit 3 = measuring (should be 0 when done)
    // Bit 0 = new_data (should be 1 when ready)
}
```

**Verify Chip ID:**

```rust
let mut chip_id = [0u8; 1];
if i2c.blocking_write_read(bme688_addr, &[BME688_REG_CHIP_ID], &mut chip_id).is_ok() {
    if chip_id[0] == 0x61 {
        info!("BME688 detected");
    } else {
        error!("Wrong chip ID: 0x{:02X}", chip_id[0]);
    }
}
```

### Radio HardFaults / Bus Faults

**Symptoms:**

- Firmware crashes with HardFault when accessing SubGHz radio
- Bus fault errors at addresses like `0x58021808` (SUBGHZSPI_SR)
- Radio initialization fails immediately
- Error: "Precise data access error" when accessing SubGHz registers

**Root Cause:**
STM32WL55 boards come with **SPI3 security enabled by default**, which blocks M4 core access to SubGHz radio registers.

**Solution:**

1. Run the security configuration script:
   ```bash
   ./disable_security.sh
   ```
2. This disables SPI3 security (`SUBGHSPISD=0x1`), allowing M4 core to access SubGHz radio
3. See `SECURITY_SETUP.md` for detailed instructions

**Note**: After disabling security, you can use `stm32wlxx-hal` to access SubGHz radio from M4 core. See `LORAWAN_INTEGRATION.md` for integration steps.

**Related Files:**

- `SECURITY_SETUP.md` - Security configuration guide
- `RADIO_NOTE.md` - Radio driver architecture notes
- `LORAWAN_INTEGRATION.md` - LoRaWAN integration plan

### Gateway Channel Configuration Issue

**Symptoms:**

- Device reports successful TX completion
- Device transmits on channel 6 (916.4 MHz) with SF10
- Gateway logs show `rxnb:0, rxok:0` (no packets received)
- RF switch is configured correctly
- Antenna is connected

**Root Cause:**
The gateway is likely **not listening on AU915 sub-band 0 (channels 0-7)**. The device transmits on channels 0-7 (915.2-916.6 MHz), but the gateway might be configured for different channels.

**Solution:**

1. **Check Gateway Channel Configuration:**

   - Access gateway web interface (usually `http://<gateway-ip>`)
   - Navigate to "Radio" or "Channel" settings
   - Verify AU915 sub-band 0 is enabled (channels 0-7: 915.2-916.6 MHz)
   - Check channel mask or enabled channels list

2. **For WisGate Edge Lite 2 (RAK7268V2):**

   - Check `global_conf.json` or web UI
   - Look for `radio_conf` section
   - Verify `chan_multiSF_X` arrays include channels 0-7
   - Example for sub-band 0:
     ```json
     "chan_multiSF_0": [915.2, 915.4, 915.6, 915.8, 916.0, 916.2, 916.4, 916.6],
     ```

3. **Verify Gateway is Listening:**

   - Gateway should show active channels in status/logs
   - Check if gateway receives packets from other devices
   - Verify gateway radio frontend is working

4. **Alternative: Change Device to Match Gateway:**
   - If gateway is configured for different sub-band, update device configuration
   - Change `Subband::_1` to match gateway's sub-band in `src/main.rs`
   - Rebuild and flash

**Device Transmission Details:**

- **Frequency**: 916.4 MHz (channel 6, sub-band 0)
- **Spreading Factor**: SF10 (corrected from SF12)
- **Bandwidth**: 125 kHz
- **Power**: 22dBm (HP mode)
- **RF Switch**: PC3=HIGH, PC4=LOW, PC5=HIGH (TX HP mode)

**Gateway Should Listen On:**

- **Sub-band 0**: Channels 0-7 (915.2, 915.4, 915.6, 915.8, 916.0, 916.2, 916.4, 916.6 MHz)
- **Data Rate**: DR2 (SF10/BW125) for join requests
- **All 8 channels** should be enabled for multi-SF reception

## Getting Help

1. **Check Logs**: Enable defmt logging and check RTT output
2. **Verify Hardware**: Double-check all connections
3. **Test Components**: Test BME688 and OLED separately
4. **Review Code**: Check initialization sequence matches datasheet
5. **Security Settings**: If radio access fails, check security configuration (see above)
6. **Community**: Check STM32/Embassy forums for similar issues

## Useful Commands

```bash
# Build and flash
cargo run --release

# Check compilation only
cargo check --target thumbv7em-none-eabihf

# View logs (requires probe-rs RTT)
probe-rs rtt

# Clean build
cargo clean
cargo build --release
```

## Known Issues

1. **No Calibration Coefficients**: Readings may drift over time
2. **Gas Sensor Raw Values**: Gas resistance shown as raw 10-bit value, not converted to kΩ
3. **Fixed Update Rate**: Cannot change 2-second interval without code modification
4. **Limited Error Recovery**: I2C errors may cause missed readings
5. **Integer Precision**: Temperature/humidity limited to whole numbers
6. **Gas Sensor Calibration**: Requires reading calibration coefficients for accurate kΩ conversion

## Contact

For issues specific to this project, check the code comments or create an issue in the repository.
