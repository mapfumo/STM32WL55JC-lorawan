# STM32WL55 Security Configuration

## Problem

STM32WL55 boards come with security enabled by default, which can prevent:
- SubGHz radio access (SPI3 security)
- Debugging/programming
- Flash write operations

## Solution: Disable Security

Based on [this blog post](https://zee-nix.blogspot.com/2025/03/making-stm32wl55-work-with-rust-i.html), you need to disable security using STM32CubeProgrammer.

### Prerequisites

- STM32CubeProgrammer installed
- Board connected via USB

### Security Configuration Script

A ready-to-use script `disable_security.sh` is provided in the project root. To use it:

```bash
#!/bin/bash

# Helper function to write to option bytes
write(){
  str=""
  for arg do
    str+=" ${arg}"
  done
  STM32_Programmer_CLI -c port=SWD mode=UR -q -ob "${str}"
}

# Path to STM32CubeProgrammer (adjust as needed)
# On Linux: /home/user/STMicroelectronics/STM32Cube/STM32CubeProgrammer/bin/STM32_Programmer_CLI
# On Windows: C:\Program Files\STMicroelectronics\STM32Cube\STM32CubeProgrammer\bin\STM32_Programmer_CLI.exe

echo "=== Disabling STM32WL55 Security ==="

echo "RDP: Read Out protection Level 1"
write RDP=0xBB

echo "RDP+ESE: Read Out protection Level 0 + Security disabled"
write RDP=0xAA ESE=0x0

echo "WRP: Write Protection disabled"
write WRP1A_STRT=0x7F WRP1A_END=0x0 WRP1B_STRT=0x7F WRP1B_END=0x0

echo "------ User Configuration ------"
echo "nRST: No reset generated when entering the Stop/Standby/Shutdown modes"
write nRST_STOP=0x1 nRST_STDBY=0x1 nRST_SHDW=0x1

echo "WDG_SW: Software window/independent watchdogs"
write WWDG_SW=0x1 IWDG_SW=0x1

echo "IWDG: Independent watchdog counter frozen in Stop/Standby modes"
write IWGD_STDBY=0x0 IWDG_STOP=0x0

echo "BOOT: CPU1+CPU2 CM0+ Boot lock disabled"
write BOOT_LOCK=0x0 C2BOOT_LOCK=0x0

echo "------ Security Configuration ------"
echo "HDPAD: User Flash hide protection area access disabled"
write HDPAD=0x1

echo "SPISD: SPI3 security disabled (CRITICAL for SubGHz radio)"
write SUBGHSPISD=0x1

echo "SBRSA: Reset default value of SRAM Start address secure"
write SNBRSA=0x1F SBRSA=0x1F

echo "SBRV: Reset default value of CPU2 Boot start address"
write SBRV=0x8000

echo "=== Security Configuration Complete ==="
```

### Usage

1. Ensure STM32CubeProgrammer is installed and board is connected via USB

2. Run the script:
   ```bash
   ./disable_security.sh
   ```

   The script will automatically find STM32CubeProgrammer in common locations. If it's not found, set the path:
   ```bash
   export STM32_PROGRAMMER_CLI="/path/to/STM32_Programmer_CLI"
   ./disable_security.sh
   ```

3. The script will disable all security settings, including the critical SPI3 security

### Critical Setting for SubGHz Radio

The most important setting for SubGHz radio access is:
```bash
write SUBGHSPISD=0x1  # SPI3 security disabled
```

Without this, the M4 core cannot access SPI3 (SubGHz radio), which will cause:
- HardFaults when accessing SubGHz registers
- Bus faults at addresses like `0x58021808` (SUBGHZSPI_SR)
- Radio initialization failures

## Flash Algorithm Issue

If you encounter this error:
```
Error: The flashing procedure failed...
Trying to write flash, but found more than one suitable flash loader algorithm...
```

Run:
```bash
target-gen arm -f "STM32WLxx_DFP"
```

This generates a corrected target definition file that probe-rs can use.

## References

- [Making STM32WL55 work with Rust](https://zee-nix.blogspot.com/2025/03/making-stm32wl55-work-with-rust-i.html)
- [STM32CubeProgrammer Documentation](https://www.st.com/en/development-tools/stm32cubeprog.html)
