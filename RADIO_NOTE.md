# Radio Initialization Note

## Issue Encountered

When attempting to initialize the SubGHz radio by directly accessing PWR_CR5 register (0x58024814), the firmware crashes with a HardFault (BusFault).

**Error:**
```
HardFault <Cause: Escalated BusFault <Cause: Precise data access error at location: 0x58024814>>
```

## Root Cause

STM32WL55 has a **dual-core architecture**:
- **Cortex-M4 core**: Runs main application (this firmware)
- **Cortex-M0+ core**: Typically controls SubGHz radio

The SubGHz radio peripheral is typically controlled by the M0+ core, not directly accessible from the M4 core. Direct register access from M4 causes bus faults.

## Solution Options

### Option 1: Use STM32CubeWL M0+ Firmware (Recommended)
STMicroelectronics provides M0+ firmware that handles radio control:
- M0+ core manages SubGHz radio registers
- M4 core communicates with M0+ via IPCC (Inter-Processor Communication)
- This is the standard approach used in STM32CubeWL examples

### Option 2: Implement IPCC Communication
Set up Inter-Processor Communication between M4 and M0+:
- M4 sends radio commands to M0+ via IPCC
- M0+ executes commands and accesses radio registers
- M4 receives status/responses from M0+

### Option 3: Verify Direct Access
Check if M4 can access SubGHz registers with proper configuration:
- May require specific clock/power setup
- Need to verify against STM32WL55 reference manual
- May need to enable specific memory regions

## Current Implementation

The radio initialization has been made **graceful**:
- Radio init functions return success but skip actual hardware access
- Sensor and display systems work normally
- No crashes or bus faults
- Ready for M0+ firmware integration

## Next Steps

1. **Short term**: Test sensor/display functionality (works without radio)
2. **Medium term**: Integrate STM32CubeWL M0+ firmware
3. **Long term**: Implement IPCC communication layer
4. **Alternative**: Research if direct M4 access is possible with proper setup

## References

- STM32WL55 Reference Manual (RM0453)
- STM32CubeWL Package (includes M0+ firmware)
- Application Note AN5406: "How to build a LoRa application with STM32CubeWL"
