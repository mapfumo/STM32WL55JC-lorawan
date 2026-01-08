#!/bin/bash

# STM32WL55 Security Configuration Script
# Based on: https://zee-nix.blogspot.com/2025/03/making-stm32wl55-work-with-rust-i.html
#
# This script disables security settings on STM32WL55 to allow:
# - SubGHz radio access (SPI3 security)
# - Debugging/programming
# - Flash write operations
#
# Prerequisites:
# - STM32CubeProgrammer installed
# - Board connected via USB
# - Edit STM32_PROGRAMMER_CLI path below if needed

set -e

# Helper function to write to option bytes
# Each option byte write needs to be a separate command
write(){
  "${STM32_PROGRAMMER_CLI}" -c port=SWD mode=UR -q -ob "$@"
}

# Find STM32CubeProgrammer CLI
# Try common locations
if [ -f "/home/$USER/STMicroelectronics/STM32Cube/STM32CubeProgrammer/bin/STM32_Programmer_CLI" ]; then
    STM32_PROGRAMMER_CLI="/home/$USER/STMicroelectronics/STM32Cube/STM32CubeProgrammer/bin/STM32_Programmer_CLI"
elif [ -f "$HOME/STMicroelectronics/STM32Cube/STM32CubeProgrammer/bin/STM32_Programmer_CLI" ]; then
    STM32_PROGRAMMER_CLI="$HOME/STMicroelectronics/STM32Cube/STM32CubeProgrammer/bin/STM32_Programmer_CLI"
elif [ -f "/opt/st/stm32cube/stm32cubeide_1.15.1/plugins/com.st.stm32cube.ide.mcu.externaltools.cubeprogrammer.linux64_2.16.0/tools/bin/STM32_Programmer_CLI" ]; then
    STM32_PROGRAMMER_CLI="/opt/st/stm32cube/stm32cubeide_1.15.1/plugins/com.st.stm32cube.ide.mcu.externaltools.cubeprogrammer.linux64_2.16.0/tools/bin/STM32_Programmer_CLI"
elif command -v STM32_Programmer_CLI &> /dev/null; then
    STM32_PROGRAMMER_CLI="STM32_Programmer_CLI"
else
    echo "ERROR: STM32_Programmer_CLI not found!"
    echo "Please install STM32CubeProgrammer or set STM32_PROGRAMMER_CLI environment variable"
    echo ""
    echo "Example:"
    echo "  export STM32_PROGRAMMER_CLI=\"/path/to/STM32_Programmer_CLI\""
    echo "  $0"
    exit 1
fi

echo "Using: $STM32_PROGRAMMER_CLI"
echo ""
echo "=== Disabling STM32WL55 Security ==="
echo "Make sure the board is connected via USB and in programming mode"
echo ""

# Read Out Protection
echo "RDP: Read Out protection Level 1"
write RDP=0xBB

echo "RDP+ESE: Read Out protection Level 0 + Security disabled"
write RDP=0xAA ESE=0x0

# Write Protection
echo "WRP: Write Protection disabled"
write WRP1A_STRT=0x7F WRP1A_END=0x0 WRP1B_STRT=0x7F WRP1B_END=0x0

# User Configuration
echo "------ User Configuration ------"
echo "nRST: No reset generated when entering the Stop/Standby/Shutdown modes"
write nRST_STOP=0x1 nRST_STDBY=0x1 nRST_SHDW=0x1

echo "WDG_SW: Software window/independent watchdogs"
write WWDG_SW=0x1 IWDG_SW=0x1

echo "IWDG: Independent watchdog counter frozen in Stop/Standby modes"
write IWGD_STDBY=0x0 IWDG_STOP=0x0

echo "BOOT: CPU1+CPU2 CM0+ Boot lock disabled"
write BOOT_LOCK=0x0 C2BOOT_LOCK=0x0

# Security Configuration
echo "------ Security Configuration ------"
echo "HDPAD: User Flash hide protection area access disabled"
write HDPAD=0x1

echo "SPISD: SPI3 security disabled (CRITICAL for SubGHz radio)"
write SUBGHZSPISD=0x1

echo "SBRSA: Reset default value of SRAM Start address secure"
write SNBRSA=0x1F SBRSA=0x1F

echo "SBRV: Reset default value of CPU2 Boot start address"
write SBRV=0x8000

echo ""
echo "=== Security Configuration Complete ==="
echo ""
echo "Next steps:"
echo "1. If you get flash algorithm errors, run: target-gen arm -f \"STM32WLxx_DFP\""
echo "2. Try flashing your firmware again"
echo "3. SubGHz radio should now be accessible from M4 core"
