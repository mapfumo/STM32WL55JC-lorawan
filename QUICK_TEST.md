# Quick Test Guide

## Ready to Test!

The firmware is ready for hardware testing. Here's what to expect:

## Build and Flash

```bash
cd /home/tony/dev/nucleo-wl55jc1/lora_1
cargo run --release
```

This will:
1. Build the firmware (optimized for size)
2. Flash to STM32WL55 via probe-rs
3. Start RTT logging

## What You Should See

### On OLED Display (5 lines):
```
STM32WL55 Node1
T:25C  H:64%
P:984 hPa
Gas: 512 (raw)
LoRa: NotJoined
```

### In RTT Logs (expected sequence):

**1. Startup:**
```
====================================
  STM32WL55 - I2C2 OLED + BME688
  Node 1 - Temp/Hum/Press/Gas
====================================
STM32WL55 initialized
```

**2. I2C Scan:**
```
Testing I2C2: PA12 (SCL), PA11 (SDA)
Scanning I2C2 bus (full scan)...
✓ Device at 0x3C
✓ Device at 0x76
Total devices found: 2
```

**3. BME688 Init:**
```
BME688 initialized @ 0x76 (gas sensor enabled)
```

**4. LoRaWAN Credentials:**
```
LoRaWAN Configuration (AU915 - 915 MHz):
  Device EUI: [hex values]
  App EUI: [hex values]
  App Key: [hex values]
```

**5. Radio Initialization:**
```
Initializing SubGHz radio for LoRaWAN...
Initializing SubGHz radio...
SubGHz radio power and clock enabled
SUBGHZSPI peripheral configured (Master mode, 8-bit, Mode 0)
Testing SPI communication...
SPI Status Register: 0x00000002
SPI CR1 Register: 0x00000047
SPI status register looks valid
Testing SPI write operation...
✓ SPI write operation successful
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

**6. Status Summary:**
```
LoRaWAN integration status:
  Sensor system: ✓ Operational
  Display system: ✓ Operational
  LoRaWAN credentials: ✓ Configured (AU915)
  Radio driver: ⏳ Basic initialization complete
  Next steps: Radio register configuration, lora-phy integration
```

**7. Main Loop (repeating every 2 seconds):**
```
✓ BME688: 25°C, 64% RH, 98400 Pa, Gas: 512
```

## Success Indicators

✅ **All Good If:**
- OLED shows sensor readings updating
- LoRa status shows "NotJoined" (normal - join not implemented yet)
- No error messages in logs
- Radio initialization completes successfully

⚠️ **Potential Issues:**

1. **"SPI Status Register: 0xFFFFFFFF"**
   - May indicate register access issue
   - Check power/clock enable

2. **"✗ Radio chip not responding"**
   - Radio chip may not be connected/responding
   - Check BUSY pin handling (currently using delays)
   - May need GPIO polling implementation

3. **"✗ SPI communication test failed"**
   - SPI peripheral may not be configured correctly
   - Check SUBGHZSPI register values

## Next Steps After Testing

Once basic functionality is verified:
1. ✅ Test SPI communication (current step)
2. ⏳ Implement BUSY pin GPIO polling (if needed)
3. ⏳ Implement `lora-phy` Radio trait
4. ⏳ Integrate with `lorawan-device` stack
5. ⏳ Test LoRaWAN join and transmission

## Notes

- Radio chip uses delay-based BUSY handling (may need GPIO polling for reliability)
- LoRaWAN join process not yet implemented (status will show "NotJoined")
- Radio TX/RX not yet implemented (lora-phy trait pending)
- Current focus: Verify initialization and SPI communication
