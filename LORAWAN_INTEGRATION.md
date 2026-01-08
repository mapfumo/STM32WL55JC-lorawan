# LoRaWAN Integration Plan

## Current Status

**LoRa is NOT working** - Current implementation is placeholder only:
- Status: `NotJoined` (hardcoded)
- SNR: 0 (hardcoded, not real)
- RSSI: 0 (hardcoded, not real)
- TX count: 0 (no transmissions)

## Root Cause

We're using `embassy-stm32` which **doesn't have SubGHz radio support** for STM32WL55. Our custom `radio.rs` implementation attempts direct register access which causes HardFaults because:

1. **SPI3 Security Enabled**: STM32WL55 boards come with SPI3 security enabled by default, which blocks M4 core access to SubGHz radio registers. This is the **primary cause** of HardFaults.

2. **Dual-Core Architecture**: STM32WL55 has M4 + M0+ cores, and SubGHz radio is typically controlled by M0+ core, but with security disabled, M4 can access it directly.

3. **Missing HAL Support**: `embassy-stm32` doesn't expose SubGHz radio driver, requiring custom implementation or use of `stm32wlxx-hal`.

**IMPORTANT**: Before attempting SubGHz radio access, you **must** disable SPI3 security using STM32CubeProgrammer. See `SECURITY_SETUP.md` for instructions.

## Solution: Use `stm32wlxx-hal`

The [`stm32wlxx-hal`](https://github.com/stm32-rs/stm32wlxx-hal) crate provides **complete SubGHz support** and works from M4 core.

### Key Findings from stm32wlxx-hal

1. **SubGHz Type**: `SubGhz<MISO, MOSI>` - Full radio driver
2. **Initialization**: 
   ```rust
   let sg = SubGhz::new(dp.SPI3, &mut dp.RCC);
   // or with DMA:
   let sg = SubGhz::new_with_dma(dp.SPI3, dma1, dma2, &mut dp.RCC);
   ```
3. **Works on M4**: Testsuite examples run on M4 core
4. **Complete API**: LoRa modulation, packet params, TX/RX, etc.

### Example Initialization Sequence (from stm32wlxx-hal testsuite)

```rust
// 1. Set standby mode
sg.set_standby(StandbyClk::Rc)?;

// 2. Configure TCXO (temperature compensated crystal oscillator)
sg.set_tcxo_mode(&TCXO_MODE)?;
sg.set_standby(StandbyClk::Hse)?;
sg.set_tx_rx_fallback_mode(FallbackMode::StandbyHse)?;

// 3. Configure regulator
sg.set_regulator_mode(RegMode::Ldo)?;

// 4. Set buffer addresses
sg.set_buffer_base_address(TX_BUF_OFFSET, RX_BUF_OFFSET)?;

// 5. Configure PA (power amplifier)
sg.set_pa_config(&PA_CONFIG)?;
sg.set_pa_ocp(Ocp::Max60m)?;
sg.set_tx_params(&TX_PARAMS)?;

// 6. Set packet type (LoRa)
sg.set_packet_type(PacketType::LoRa)?;

// 7. Configure LoRa modulation parameters
sg.set_lora_mod_params(&LORA_MOD_PARAMS)?;  // SF7, BW=125kHz, CR=4/5

// 8. Configure LoRa packet parameters
sg.set_lora_packet_params(&LORA_PACKET_PARAMS)?;  // Preamble, CRC, etc.

// 9. Calibrate for frequency band
sg.calibrate_image(CalibrateImage::ISM_915_917)?;  // For AU915

// 10. Set RF frequency
sg.set_rf_frequency(&RfFreq::from_frequency(915_200_000))?;  // 915.2 MHz
```

## Integration Options

### Option 1: Use stm32wlxx-hal for SubGHz Only (Recommended)

Keep `embassy-stm32` for I2C, GPIO, timers, etc., but use `stm32wlxx-hal` for SubGHz:

**Pros:**
- Minimal changes to existing code
- Keep embassy async framework
- Use proven SubGHz implementation

**Cons:**
- Need to access PAC directly for SubGHz
- Two HALs in same project (but compatible)

**Implementation:**
1. Add `stm32wlxx-hal` dependency with `stm32wl5x_cm4` feature
2. Access PAC via `embassy-stm32::pac` or `stm32wlxx_hal::pac`
3. Create `SubGhz` instance alongside embassy peripherals
4. Replace our custom `radio.rs` with `stm32wlxx-hal::SubGhz`

### Option 2: Migrate Entirely to stm32wlxx-hal

Replace `embassy-stm32` with `stm32wlxx-hal` for all peripherals.

**Pros:**
- Single HAL, consistent API
- Full SubGHz support

**Cons:**
- Major refactor (lose embassy async)
- Need to rewrite I2C, GPIO, etc.
- More work

### Option 3: Wait for embassy-stm32 SubGHz Support

Wait for `embassy-stm32` to add SubGHz support (may never happen).

## Recommended Path: Option 1

### Step 1: Add stm32wlxx-hal Dependency

```toml
[dependencies]
stm32wlxx-hal = { version = "0.6.1", features = [
    "stm32wl5x_cm4",  # M4 core
    "defmt",          # For logging
] }
```

### Step 2: Access PAC for SubGHz

```rust
use embassy_stm32::pac;
use stm32wlxx_hal::subghz::SubGhz;

// Get PAC from embassy
let pac = embassy_stm32::pac::Peripherals::steal();
let rcc = &mut pac.RCC;

// Create SubGhz instance
let mut subghz = SubGhz::new(pac.SPI3, rcc)?;
```

### Step 3: Initialize SubGHz for AU915

```rust
// Initialize sequence (see example above)
subghz.set_standby(StandbyClk::Rc)?;
subghz.set_tcxo_mode(&TCXO_MODE)?;
// ... rest of initialization
subghz.set_rf_frequency(&RfFreq::from_frequency(915_200_000))?;
```

### Step 4: Integrate with lora-phy

`stm32wlxx-hal::SubGhz` can be wrapped to implement `lora-phy::Radio` trait, then used with `lorawan-device`.

## Next Steps

1. ✅ Research stm32wlxx-hal (done)
2. ⏳ Add stm32wlxx-hal dependency
3. ⏳ Create SubGhz instance alongside embassy peripherals
4. ⏳ Initialize SubGHz for AU915 band
5. ⏳ Test basic TX/RX
6. ⏳ Integrate with lora-phy Radio trait
7. ⏳ Integrate with lorawan-device stack
8. ⏳ Implement OTAA join
9. ⏳ Send sensor data via LoRaWAN

## References

- [stm32wlxx-hal repository](https://github.com/stm32-rs/stm32wlxx-hal)
- [stm32wlxx-hal SubGHz testsuite](https://github.com/stm32-rs/stm32wlxx-hal/tree/main/testsuite/src/subghz.rs)
- [STM32WL55 Reference Manual](https://www.st.com/resource/en/reference_manual/rm0453-stm32wl5x-advanced-armbased-32bit-mcus-stmicroelectronics.pdf)
