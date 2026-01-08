# Testing Guide

## Current Firmware Status

### What's Implemented
- ✅ BME688 sensor reading (temperature, humidity, pressure, gas resistance)
- ✅ OLED display (SH1106, 128x64)
- ✅ SubGHz radio power/clock initialization
- ✅ SUBGHZSPI peripheral configuration
- ✅ SPI communication functions
- ✅ SX126x command interface
- ✅ Radio chip initialization sequence
- ✅ LoRa modulation parameter configuration
- ✅ AU915 frequency band setup

### What to Test

#### 1. Sensor System
- **Expected**: Temperature, humidity, pressure readings displayed on OLED
- **Check**: Values should be reasonable for your environment
- **Logs**: Look for "✓ BME688: XX°C, XX% RH, XXXX Pa, Gas: XXX"

#### 2. Display System
- **Expected**: 5-line display showing:
  - Line 1: "STM32WL55 Node1"
  - Line 2: "T:XXC  H:XX%"
  - Line 3: "P:XXX hPa"
  - Line 4: "Gas: XXX (raw)"
  - Line 5: LoRa status
- **Check**: Display should update every 2 seconds

#### 3. Radio Initialization
- **Expected**: Radio initialization sequence completes successfully
- **Logs to check**:
  - "Initializing SubGHz radio..."
  - "SubGHz radio power and clock enabled"
  - "SUBGHZSPI peripheral configured"
  - "Testing SPI communication..."
  - "SPI Status Register: 0xXXXXXXXX"
  - "✓ SPI communication test passed"
  - "Initializing radio chip (SX126x series)..."
  - "✓ Radio chip responding, status: 0xXX"
  - "✓ Radio chip initialized successfully"
  - "✓ Radio configured for AU915 (915 MHz)"

#### 4. LoRa Status Display
- **Expected**: Line 5 shows LoRa status
- **Possible states**:
  - "LoRa: NotJoined" (normal after initialization)
  - "LoRa: Joining..." (during join process)
  - "LoRa: SNR:X TX:Y" (after successful join)
  - "LoRa: Error" (if initialization failed)

## Testing Procedure

### Prerequisites
1. Hardware connected:
   - BME688 sensor on I2C2 (PA11/PA12)
   - SH1106 OLED on I2C2 (PA11/PA12)
   - STM32WL55 board powered
   - Probe-rs connected for flashing and RTT

2. Software ready:
   - `probe-rs` installed
   - RTT viewer or serial monitor ready

### Step 1: Build and Flash
```bash
cd /home/tony/dev/nucleo-wl55jc1/lora_1
cargo run --release
```

This will:
- Build the firmware
- Flash to STM32WL55
- Start RTT logging

### Step 2: Monitor Output
Watch for initialization sequence:

1. **Startup**:
   ```
   ====================================
     STM32WL55 - I2C2 OLED + BME688
     Node 1 - Temp/Hum/Press/Gas
   ====================================
   STM32WL55 initialized
   ```

2. **I2C Scan**:
   ```
   Testing I2C2: PA12 (SCL), PA11 (SDA)
   Scanning I2C2 bus (full scan)...
   ✓ Device at 0x3C
   ✓ Device at 0x76  (or 0x77)
   Total devices found: 2
   ```

3. **BME688 Initialization**:
   ```
   BME688 initialized @ 0x76 (gas sensor enabled)
   ```

4. **LoRaWAN Credentials**:
   ```
   LoRaWAN Configuration (AU915 - 915 MHz):
     Device EUI: [hex values]
     App EUI: [hex values]
     App Key: [hex values]
   ```

5. **Radio Initialization**:
   ```
   Initializing SubGHz radio for LoRaWAN...
   Initializing SubGHz radio...
   SubGHz radio power and clock enabled
   SUBGHZSPI peripheral configured (Master mode, 8-bit, Mode 0)
   Testing SPI communication...
   SPI Status Register: 0x00000002  (or similar)
   ✓ SPI communication test passed
   Initializing radio chip (SX126x series)...
   ✓ Radio chip responding, status: 0xXX
   Setting radio to standby mode...
   Setting regulator mode...
   Calibrating radio...
   Setting packet type to LoRa...
   ✓ Radio chip initialized successfully
   Configuring radio for AU915 (915 MHz)...
   RF frequency set to 915 MHz (0xXXXXXXXX)
   LoRa modulation params: SF7, BW=125kHz, CR=4/5
   LoRa packet params: Preamble=8, Header=Explicit, Payload=0, CRC=On, IQ=Normal
   ✓ Radio configured for AU915 (915 MHz)
   ```

6. **Main Loop**:
   ```
   ✓ BME688: 25°C, 64% RH, 98400 Pa, Gas: 512
   ```

### Step 3: Verify Hardware

#### Check OLED Display
- Should show sensor readings updating every 2 seconds
- LoRa status should show "LoRa: NotJoined" (normal - not yet joined to network)

#### Check Sensor Readings
- Temperature: Should match room temperature (±2°C)
- Humidity: Should match local conditions
- Pressure: Should be reasonable for your elevation
- Gas: Raw value (0-1023), typically 100-800 for clean air

#### Check Radio Status
- If radio initialization succeeds: "LoRa: NotJoined"
- If radio initialization fails: "LoRa: Error"
- Check logs for any error messages

## Troubleshooting

### Radio Initialization - M0+ Core Requirement

**Important:** STM32WL55 has dual-core architecture (M4 + M0+). The SubGHz radio is typically controlled by the M0+ core, not the M4 core.

**Current Status:**
- Radio initialization is deferred to avoid bus faults
- Direct register access from M4 core causes HardFault
- Radio functionality requires M0+ core firmware

**Solutions:**
1. **Use STM32CubeWL M0+ Firmware**
   - STMicroelectronics provides M0+ firmware for radio control
   - M4 core communicates with M0+ via IPCC (Inter-Processor Communication)
   - This is the recommended approach

2. **Implement IPCC Communication**
   - Set up IPCC peripheral for M4 ↔ M0+ communication
   - M4 sends commands to M0+ via IPCC
   - M0+ handles radio register access

3. **Verify Register Access**
   - Check if M4 can access SubGHz registers directly
   - May require specific clock/power configuration
   - Reference manual verification needed

**Expected Behavior:**
- Radio initialization will be skipped gracefully
- Sensor and display systems will work normally
- LoRa status will show appropriate message
- No crashes or bus faults

### Sensor Readings Wrong

**See TROUBLESHOOTING.md** for sensor-specific issues.

### Display Not Working

**See TROUBLESHOOTING.md** for display-specific issues.

## Expected Behavior

### Successful Initialization
- All systems initialize without errors
- Sensor readings appear on OLED
- LoRa status shows "NotJoined" (normal - join not yet implemented)
- Logs show successful radio initialization

### Next Steps After Testing
Once basic functionality is verified:
1. Implement `lora-phy` Radio trait for TX/RX
2. Integrate with `lorawan-device` stack
3. Implement OTAA join process
4. Test LoRaWAN communication

## Test Checklist

- [ ] Firmware builds successfully
- [ ] Firmware flashes to board
- [ ] RTT output visible
- [ ] I2C devices detected (OLED + BME688)
- [ ] BME688 sensor readings appear
- [ ] OLED display shows sensor data
- [ ] Radio power/clock enabled
- [ ] SUBGHZSPI configured
- [ ] SPI communication test passes
- [ ] Radio chip responds to GetStatus command
- [ ] Radio chip initialization completes
- [ ] LoRa parameters configured
- [ ] AU915 frequency set
- [ ] LoRa status shows on display

## Notes

- Radio chip communication uses delays instead of BUSY pin polling (may need improvement)
- LoRaWAN join process not yet implemented (status will show "NotJoined")
- Radio TX/RX functionality not yet implemented (lora-phy trait pending)
- Current implementation focuses on initialization and configuration
