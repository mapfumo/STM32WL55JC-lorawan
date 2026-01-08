# Implementation Plan: Apply Working Solution RF Switch Pattern

## Recommendation: Option 2 - Keep stm32wlxx-hal, Apply RF Switch Improvements

**Why:**
- ✅ Radio initialization already works with stm32wlxx-hal
- ✅ No dependency version conflicts
- ✅ Minimal refactoring needed
- ✅ RF switch control is the key improvement from working solution

## Key Insight from Working Solution

The working solution uses `lora-phy`'s `InterfaceVariant` trait which automatically calls:
- `enable_rf_switch_tx()` before TX
- `enable_rf_switch_rx()` before RX  
- `disable_rf_switch()` after operations

**RF Switch Control Logic (from working solution):**
- **TX High Power**: PC4=Low (when `use_high_power_pa=true`), PC5=High, PC3=High
- **RX**: PC4=High, PC5=Low, PC3=High
- **Disabled**: PC3=Low, PC4=Low, PC5=Low

**Our Current Implementation:**
- ✅ TX High Power: PC3=SET, PC4=RESET, PC5=SET (matches!)
- ✅ RX: PC3=SET, PC4=SET, PC5=RESET (matches!)

## What Needs to Change

1. **Ensure RF switch is set BEFORE starting TX/RX** (not after)
2. **Remove manual delays** - RF switch settles quickly, no need for 5ms delay
3. **Match the exact timing** from working solution

## Implementation Steps

1. ✅ Revert Cargo.toml to use embassy-stm32 v0.1.0 and stm32wlxx-hal
2. ⏳ Restore original radio.rs with stm32wlxx-hal implementation
3. ⏳ Verify RF switch control matches working solution pattern
4. ⏳ Ensure RF switch is set at correct times (before TX/RX start)
5. ⏳ Test build and verify compilation
6. ⏳ Test on hardware

## Files to Restore

- `src/radio.rs` - Original stm32wlxx-hal based implementation
- `src/main.rs` - Original main.rs that uses SubGhzRadio
- `Cargo.toml` - Already reverted ✅
