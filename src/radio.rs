//! SubGHz Radio Driver for STM32WL55 using stm32wlxx-hal
//! 
//! This module wraps stm32wlxx-hal's SubGhz driver for use with lorawan-device.
//! It implements the PhyRxTx trait and handles RF switch control for NUCLEO-WL55JC1.

use defmt::*;
use embassy_time::Timer;
use lorawan_device::async_device::radio::{PhyRxTx, RxConfig, RxQuality, RxStatus, TxConfig};
use lorawan_device::async_device::Timings;
use stm32wlxx_hal::{
    pac as stm32wl_pac,
    subghz::{
        CalibrateImage, CmdStatus, CodingRate, FallbackMode, HeaderType, LoRaBandwidth, LoRaModParams,
        LoRaPacketParams, LoRaSyncWord, Ocp, PaConfig, PacketType, RegMode, RfFreq,
        SleepCfg, SpreadingFactor, StandbyClk, SubGhz, TcxoMode, TcxoTrim, Timeout, TxParams, RampTime,
    },
    spi::{SgMiso, SgMosi},
};

/// Radio state machine
#[derive(Debug, Clone, Copy, PartialEq, defmt::Format)]
pub enum RadioState {
    Idle,
    Transmitting,
    Receiving,
    Error,
}

/// Radio error types
#[derive(Debug, Clone, Copy, PartialEq, defmt::Format)]
pub enum RadioError {
    NotInitialized,
    ConfigurationFailed,
    TransmissionFailed,
    ReceptionFailed,
    InvalidParameter,
}

/// Convert lora_modulation SpreadingFactor to stm32wlxx-hal SpreadingFactor
fn convert_sf(sf: lora_modulation::SpreadingFactor) -> SpreadingFactor {
    match sf {
        lora_modulation::SpreadingFactor::_5 => SpreadingFactor::Sf5,
        lora_modulation::SpreadingFactor::_6 => SpreadingFactor::Sf6,
        lora_modulation::SpreadingFactor::_7 => SpreadingFactor::Sf7,
        lora_modulation::SpreadingFactor::_8 => SpreadingFactor::Sf8,
        lora_modulation::SpreadingFactor::_9 => SpreadingFactor::Sf9,
        lora_modulation::SpreadingFactor::_10 => SpreadingFactor::Sf10,
        lora_modulation::SpreadingFactor::_11 => SpreadingFactor::Sf11,
        lora_modulation::SpreadingFactor::_12 => SpreadingFactor::Sf12,
    }
}

/// Convert lora_modulation Bandwidth to stm32wlxx-hal LoRaBandwidth
fn convert_bw(bw: lora_modulation::Bandwidth) -> Result<LoRaBandwidth, RadioError> {
    match bw {
        lora_modulation::Bandwidth::_125KHz => Ok(LoRaBandwidth::Bw125),
        lora_modulation::Bandwidth::_250KHz => Ok(LoRaBandwidth::Bw250),
        lora_modulation::Bandwidth::_500KHz => Ok(LoRaBandwidth::Bw500),
        _ => Err(RadioError::InvalidParameter),
    }
}

/// Convert lora_modulation CodingRate to stm32wlxx-hal CodingRate
fn convert_cr(cr: lora_modulation::CodingRate) -> CodingRate {
    match cr {
        lora_modulation::CodingRate::_4_5 => CodingRate::Cr45,
        lora_modulation::CodingRate::_4_6 => CodingRate::Cr46,
        lora_modulation::CodingRate::_4_7 => CodingRate::Cr47,
        lora_modulation::CodingRate::_4_8 => CodingRate::Cr48,
    }
}

/// RF Switch control for NUCLEO-WL55JC1 board
/// Controls GPIO pins PC3, PC4, PC5 to route RF signals to/from antenna
/// Based on working solution pattern from lora_known_working
struct RfSwitch {
    gpio_c: Option<stm32wl_pac::GPIOC>,
}

impl RfSwitch {
    fn new(gpio_c: stm32wl_pac::GPIOC) -> Self {
        let mut sw = Self { gpio_c: Some(gpio_c) };
        sw.init();
        sw
    }

    fn init(&mut self) {
        if let Some(ref gpio_c) = self.gpio_c {
            // Enable GPIOC clock
            let rcc = unsafe { &*stm32wl_pac::RCC::ptr() };
            rcc.ahb2enr.modify(|_, w| w.gpiocen().set_bit());
            
            // Configure PC3, PC4, PC5 as output, push-pull, high speed
            gpio_c.moder.modify(|_, w| {
                w.moder3().bits(0b01)  // PC3 = output
                 .moder4().bits(0b01)  // PC4 = output
                 .moder5().bits(0b01)  // PC5 = output
            });
            gpio_c.otyper.modify(|_, w| {
                w.ot3().clear_bit()  // PC3 = push-pull
                 .ot4().clear_bit()  // PC4 = push-pull
                 .ot5().clear_bit()  // PC5 = push-pull
            });
            gpio_c.ospeedr.modify(|_, w| {
                w.ospeedr3().bits(0b11)  // PC3 = very high speed
                 .ospeedr4().bits(0b11)  // PC4 = very high speed
                 .ospeedr5().bits(0b11)  // PC5 = very high speed
            });
            
            // Initialize RF switch pins to OFF state
            // Note: Working solution initializes PC3 to HIGH, but we start with all LOW
            // and set them appropriately before TX/RX operations
            gpio_c.bsrr.write(|w| {
                w.br3().set_bit()  // PC3 = reset (low) - will be set HIGH before TX/RX
                 .br4().set_bit()  // PC4 = reset (low)
                 .br5().set_bit()  // PC5 = reset (low)
            });
        }
    }

    /// Configure RF switch for RX mode: PC3=SET, PC4=SET, PC5=RESET
    /// Matches working solution: enable_rf_switch_rx()
    fn set_rx(&mut self) {
        if let Some(ref gpio_c) = self.gpio_c {
            gpio_c.bsrr.write(|w| {
                w.bs3().set_bit()  // PC3 = set (high)
                 .bs4().set_bit()  // PC4 = set (high)
                 .br5().set_bit()  // PC5 = reset (low)
            });
        }
    }

    /// Configure RF switch for TX High Power: PC3=SET, PC4=RESET, PC5=SET
    /// Matches working solution: enable_rf_switch_tx() with use_high_power_pa=true
    fn set_tx_hp(&mut self) {
        if let Some(ref gpio_c) = self.gpio_c {
            gpio_c.bsrr.write(|w| {
                w.bs3().set_bit()  // PC3 = set (high)
                 .br4().set_bit()  // PC4 = reset (low)
                 .bs5().set_bit()  // PC5 = set (high)
            });
        }
    }

    /// Turn off RF switch: PC3=RESET, PC4=RESET, PC5=RESET
    /// Matches working solution: disable_rf_switch()
    fn set_off(&mut self) {
        if let Some(ref gpio_c) = self.gpio_c {
            gpio_c.bsrr.write(|w| {
                w.br3().set_bit()  // PC3 = reset (low)
                 .br4().set_bit()  // PC4 = reset (low)
                 .br5().set_bit()  // PC5 = reset (low)
            });
        }
    }
}

/// Radio driver structure wrapping stm32wlxx-hal SubGhz
pub struct SubGhzRadio {
    subghz: Option<SubGhz<SgMiso, SgMosi>>,
    rf_switch: Option<RfSwitch>,
    state: RadioState,
    initialized: bool,
    frequency_mhz: u32,
    last_snr: i8,
    last_rssi: i16,
    rx_setup: bool,
}

impl SubGhzRadio {
    /// Create a new radio driver instance
    pub fn new() -> Self {
        Self {
            subghz: None,
            rf_switch: None,
            state: RadioState::Idle,
            initialized: false,
            frequency_mhz: 0,
            last_snr: 0,
            last_rssi: 0,
            rx_setup: false,
        }
    }

    /// Initialize the SubGHz radio using stm32wlxx-hal
    pub fn init(
        &mut self,
        spi3: stm32wl_pac::SPI3,
        rcc: &mut stm32wl_pac::RCC,
        gpio_c: stm32wl_pac::GPIOC,
    ) -> Result<(), RadioError> {
        info!("Initializing SubGHz radio with stm32wlxx-hal...");

        // Initialize RF switch (NUCLEO-WL55JC1 board requirement)
        let rf_switch = RfSwitch::new(gpio_c);
        self.rf_switch = Some(rf_switch);
        info!("  RF switch initialized (PC3, PC4, PC5)");

        // Create SubGhz instance
        let subghz = SubGhz::new(spi3, rcc);
        
        self.subghz = Some(subghz);
        self.initialized = true;
        self.state = RadioState::Idle;

        info!("✓ SubGHz radio initialized");
        Ok(())
    }

    /// Configure radio for AU915 frequency band (915 MHz)
    pub async fn configure_au915(&mut self) -> Result<(), RadioError> {
        if !self.initialized {
            return Err(RadioError::NotInitialized);
        }

        let subghz = self.subghz.as_mut().ok_or(RadioError::NotInitialized)?;
        info!("Configuring radio for AU915 (915 MHz)...");

        // 1. Set standby mode (RC)
        subghz.set_standby(StandbyClk::Rc)
            .map_err(|_| RadioError::ConfigurationFailed)?;
        info!("  Standby mode set (RC)");

        // 2. Configure TCXO
        // Timeout value 0x1F = 31 * 15.625us ≈ 487us (minimum timeout)
        // For ~62ms, we'd need a larger value, but 0x1F is a reasonable default
        let tcxo_timeout = Timeout::from_millis_sat(1); // 1ms timeout (radio will handle longer)
        let tcxo_mode = TcxoMode::new()
            .set_txco_trim(TcxoTrim::Volts1pt7)
            .set_timeout(tcxo_timeout);
        subghz.set_tcxo_mode(&tcxo_mode)
            .map_err(|_| RadioError::ConfigurationFailed)?;
        info!("  TCXO configured");

        // 3. Set standby mode (HSE) - required after TCXO config
        subghz.set_standby(StandbyClk::Hse)
            .map_err(|_| RadioError::ConfigurationFailed)?;
        info!("  Standby mode set (HSE)");

        // 4. Set fallback mode
        subghz.set_tx_rx_fallback_mode(FallbackMode::StandbyHse)
            .map_err(|_| RadioError::ConfigurationFailed)?;

        // 5. Configure regulator
        subghz.set_regulator_mode(RegMode::Ldo)
            .map_err(|_| RadioError::ConfigurationFailed)?;
        info!("  Regulator mode: LDO");

        // 6. Set buffer addresses
        const TX_BUF_OFFSET: u8 = 128;
        const RX_BUF_OFFSET: u8 = 0;
        subghz.set_buffer_base_address(TX_BUF_OFFSET, RX_BUF_OFFSET)
            .map_err(|_| RadioError::ConfigurationFailed)?;
        info!("  Buffer addresses: TX={}, RX={}", TX_BUF_OFFSET, RX_BUF_OFFSET);

        // 7. Configure PA (Power Amplifier) for high power
        let pa_config = PaConfig::HP_22;
        subghz.set_pa_config(&pa_config)
            .map_err(|_| RadioError::ConfigurationFailed)?;
        subghz.set_pa_ocp(Ocp::Max60m)
            .map_err(|_| RadioError::ConfigurationFailed)?;
        info!("  PA configured: HP_22DBM (22dBm)");

        // 8. Configure TX parameters
        let tx_params = TxParams::HP.set_ramp_time(RampTime::Micros40);
        subghz.set_tx_params(&tx_params)
            .map_err(|_| RadioError::ConfigurationFailed)?;
        info!("  TX parameters configured");

        // 9. Set packet type (LoRa)
        subghz.set_packet_type(PacketType::LoRa)
            .map_err(|_| RadioError::ConfigurationFailed)?;
        info!("  Packet type: LoRa");

        // 10. Configure LoRa modulation (default: SF7, BW=125kHz, CR=4/5)
        // This will be updated per transmission by lorawan-device
        let lora_mod_params = LoRaModParams::new()
            .set_bw(LoRaBandwidth::Bw125)
            .set_cr(CodingRate::Cr45)
            .set_ldro_en(false)
            .set_sf(SpreadingFactor::Sf7);
        subghz.set_lora_mod_params(&lora_mod_params)
            .map_err(|_| RadioError::ConfigurationFailed)?;
        info!("  LoRa modulation: SF7, BW=125kHz, CR=4/5");

        // 11. Configure LoRa packet parameters
        const PREAMBLE_LEN: u16 = 8;
        let lora_packet_params = LoRaPacketParams::new()
            .set_crc_en(true)
            .set_preamble_len(PREAMBLE_LEN)
            .set_payload_len(0) // Variable length
            .set_invert_iq(false)
            .set_header_type(HeaderType::Variable);
        subghz.set_lora_packet_params(&lora_packet_params)
            .map_err(|_| RadioError::ConfigurationFailed)?;
        info!("  LoRa packet: Preamble={}, Variable header, CRC enabled", PREAMBLE_LEN);

        // 12. Set sync word (Public LoRaWAN)
        subghz.set_lora_sync_word(LoRaSyncWord::Public)
            .map_err(|_| RadioError::ConfigurationFailed)?;
        info!("  LoRa sync word: Public (0x34)");

        // 13. Calibrate for ISM 902-928 MHz (AU915)
        subghz.calibrate_image(CalibrateImage::ISM_902_928)
            .map_err(|_| RadioError::ConfigurationFailed)?;
        info!("  Calibrated for ISM 902-928 MHz (AU915)");

        // 14. Set default RF frequency (915.2 MHz - channel 0)
        let freq = RfFreq::from_frequency(915_200_000);
        subghz.set_rf_frequency(&freq)
            .map_err(|_| RadioError::ConfigurationFailed)?;
        self.frequency_mhz = 915;
        info!("  RF frequency: 915.2 MHz");

        info!("✓ Radio configured for AU915:");
        info!("  Frequency: 915 MHz (915.2 MHz default)");
        info!("  Modulation: SF7, BW=125kHz, CR=4/5");
        info!("  Packet: Preamble=8, Explicit header, CRC enabled");
        Ok(())
    }

    /// Check if radio is initialized
    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    /// Get current frequency in MHz
    pub fn frequency(&self) -> u32 {
        self.frequency_mhz
    }

    /// Get current radio state
    pub fn state(&self) -> RadioState {
        self.state
    }
}

// Implement PhyRxTx trait for lorawan-device
impl PhyRxTx for SubGhzRadio {
    const MAX_RADIO_POWER: u8 = 22; // 22 dBm max for STM32WL55
    
    type PhyError = RadioError;

    async fn tx(&mut self, config: TxConfig, buf: &[u8]) -> Result<u32, Self::PhyError> {
        if !self.initialized {
            return Err(RadioError::NotInitialized);
        }

        let subghz = self.subghz.as_mut().ok_or(RadioError::NotInitialized)?;

        // Convert modulation parameters
        let bb = config.rf.bb;
        let mut sf = convert_sf(bb.sf);
        let bw = convert_bw(bb.bw)?;
        let cr = convert_cr(bb.cr);
        
        // WORKAROUND: lorawan-device v0.12 has a bug where AU915 join requests use DR0 (SF12)
        // instead of DR2 (SF10). Force SF10 for join requests.
        let sf_factor = bb.sf.factor();
        let is_125khz = bb.bw.hz() == 125_000;
        let is_uplink_freq = config.rf.frequency >= 915_200_000 && config.rf.frequency <= 927_800_000;
        
        if sf_factor == 12 && is_125khz && is_uplink_freq {
            warn!("  🔧 WORKAROUND: Detected SF12/125kHz on uplink - forcing SF10 for join request");
            sf = SpreadingFactor::Sf10;
        }
        
        // LDRO must only be enabled for SF11 and SF12
        let final_sf_factor = if sf_factor == 12 && is_125khz && is_uplink_freq {
            10
        } else {
            sf_factor
        };
        let ldro_enabled = final_sf_factor >= 11;

        // Set modulation parameters
        let lora_mod_params = LoRaModParams::new()
            .set_bw(bw)
            .set_cr(cr)
            .set_ldro_en(ldro_enabled)
            .set_sf(sf);
        subghz.set_lora_mod_params(&lora_mod_params)
            .map_err(|_| RadioError::ConfigurationFailed)?;

        // Set packet parameters
        const PREAMBLE_LEN: u16 = 8;
        let lora_packet_params = LoRaPacketParams::new()
            .set_crc_en(true)
            .set_preamble_len(PREAMBLE_LEN)
            .set_payload_len(0)
            .set_invert_iq(false)
            .set_header_type(HeaderType::Variable);
        subghz.set_lora_packet_params(&lora_packet_params)
            .map_err(|_| RadioError::ConfigurationFailed)?;

        // Configure TX power (HP mode)
        let tx_params = TxParams::HP.set_ramp_time(RampTime::Micros40);
        subghz.set_tx_params(&tx_params)
            .map_err(|_| RadioError::ConfigurationFailed)?;

        // Set frequency
        let freq = RfFreq::from_frequency(config.rf.frequency);
        subghz.set_rf_frequency(&freq)
            .map_err(|_| RadioError::ConfigurationFailed)?;
        self.frequency_mhz = (config.rf.frequency / 1_000_000) as u32;
        info!("  TX frequency: {}.{} MHz ({} Hz)", 
              config.rf.frequency / 1_000_000,
              (config.rf.frequency % 1_000_000) / 100_000,
              config.rf.frequency);

        // Write data to TX buffer
        const TX_BUF_OFFSET: u8 = 128;
        subghz.write_buffer(TX_BUF_OFFSET, buf)
            .map_err(|_| RadioError::TransmissionFailed)?;

        // Configure RF switch for TX High Power mode (CRITICAL: before TX start)
        // Matches working solution: enable_rf_switch_tx() with use_high_power_pa=true
        // Set RF switch BEFORE starting TX to ensure signal routing is correct
        // Working solution: PC4=LOW (rf_switch_rx), PC5=HIGH (rf_switch_tx), PC3=HIGH (rf_switch_en)
        if let Some(ref mut sw) = self.rf_switch {
            sw.set_tx_hp();
        }
        info!("  RF switch: TX High Power mode (PC3=HIGH, PC4=LOW, PC5=HIGH)");
        
        // Minimal delay for RF switch to settle (working solution doesn't delay, but we add small delay for safety)
        Timer::after_millis(1).await;

        // Start transmission
        subghz.set_tx(Timeout::DISABLED)
            .map_err(|_| RadioError::TransmissionFailed)?;
        
        // Verify radio entered TX mode
        let tx_status = subghz.status()
            .map_err(|_| RadioError::TransmissionFailed)?;
        info!("  TX started - Radio mode: {:?}, Cmd: {:?}", tx_status.mode(), tx_status.cmd());

        self.state = RadioState::Transmitting;

        // Wait for TX completion
        let mut timeout_count = 0;
        loop {
            let status = subghz.status()
                .map_err(|_| RadioError::TransmissionFailed)?;

            if status.cmd() == Ok(CmdStatus::Complete) {
                info!("  TX complete");

                // Return to standby
                subghz.set_standby(StandbyClk::Hse)
                    .map_err(|_| RadioError::TransmissionFailed)?;
                
                // Turn off RF switch after TX
                if let Some(ref mut sw) = self.rf_switch {
                    sw.set_off();
                }
                
                self.state = RadioState::Idle;
                return Ok(buf.len() as u32);
            }

            timeout_count += 1;
            if timeout_count > 500 {
                warn!("  TX timeout!");
                subghz.set_standby(StandbyClk::Hse)
                    .map_err(|_| RadioError::TransmissionFailed)?;
                if let Some(ref mut sw) = self.rf_switch {
                    sw.set_off();
                }
                self.state = RadioState::Idle;
                return Err(RadioError::TransmissionFailed);
            }

            Timer::after_millis(10).await;
        }
    }

    async fn setup_rx(&mut self, config: RxConfig) -> Result<(), Self::PhyError> {
        if !self.initialized {
            return Err(RadioError::NotInitialized);
        }

        let subghz = self.subghz.as_mut().ok_or(RadioError::NotInitialized)?;

        // Convert modulation parameters
        let bb = config.rf.bb;
        let sf = convert_sf(bb.sf);
        let bw = convert_bw(bb.bw)?;
        let cr = convert_cr(bb.cr);
        
        // LDRO must only be enabled for SF11 and SF12
        let sf_factor = bb.sf.factor();
        let ldro_enabled = sf_factor >= 11;

        // Set modulation parameters
        let lora_mod_params = LoRaModParams::new()
            .set_bw(bw)
            .set_cr(cr)
            .set_ldro_en(ldro_enabled)
            .set_sf(sf);
        subghz.set_lora_mod_params(&lora_mod_params)
            .map_err(|_| RadioError::ConfigurationFailed)?;

        // Set packet parameters
        const PREAMBLE_LEN: u16 = 8;
        let lora_packet_params = LoRaPacketParams::new()
            .set_crc_en(true)
            .set_preamble_len(PREAMBLE_LEN)
            .set_payload_len(0)
            .set_invert_iq(true) // Inverted IQ for downlink
            .set_header_type(HeaderType::Variable);
        subghz.set_lora_packet_params(&lora_packet_params)
            .map_err(|_| RadioError::ConfigurationFailed)?;

        // Set frequency
        let freq = RfFreq::from_frequency(config.rf.frequency);
        subghz.set_rf_frequency(&freq)
            .map_err(|_| RadioError::ConfigurationFailed)?;

        // Configure RF switch for RX mode (CRITICAL: before RX start)
        // Matches working solution: enable_rf_switch_rx()
        if let Some(ref mut sw) = self.rf_switch {
            sw.set_rx();
        }
        info!("  RF switch: RX mode");

        self.rx_setup = true;
        Ok(())
    }

    async fn rx_single(&mut self, buf: &mut [u8]) -> Result<RxStatus, Self::PhyError> {
        if !self.initialized || !self.rx_setup {
            return Err(RadioError::NotInitialized);
        }

        let subghz = self.subghz.as_mut().ok_or(RadioError::NotInitialized)?;

        // Start RX with default timeout (5 seconds for RX windows)
        let timeout = Timeout::from_millis_sat(5000);
        subghz.set_rx(timeout)
            .map_err(|_| RadioError::ReceptionFailed)?;

        self.state = RadioState::Receiving;

        loop {
            let status = subghz.status()
                .map_err(|_| RadioError::ReceptionFailed)?;

            if status.cmd() == Ok(CmdStatus::Complete) {
                // Read packet status
                let pkt_status = subghz.lora_packet_status()
                    .map_err(|_| RadioError::ReceptionFailed)?;
                
                self.last_snr = pkt_status.snr_pkt().to_integer().clamp(-128, 127) as i8;
                self.last_rssi = pkt_status.rssi_pkt().to_integer();

                // Read payload
                let payload_len = buf.len().min(255);
                subghz.read_buffer(0, &mut buf[..payload_len])
                    .map_err(|_| RadioError::ReceptionFailed)?;

                // Return to standby
                subghz.set_standby(StandbyClk::Hse)
                    .map_err(|_| RadioError::ReceptionFailed)?;

                // Turn off RF switch after RX
                if let Some(ref mut sw) = self.rf_switch {
                    sw.set_off();
                }

                self.state = RadioState::Idle;
                let quality = RxQuality::new(self.last_rssi, self.last_snr);
                info!("  RX complete: {} bytes, SNR={}, RSSI={}", payload_len, self.last_snr, self.last_rssi);
                return Ok(RxStatus::Rx(payload_len, quality));
            }

            // Check if radio is still in RX mode (timeout handled by radio hardware)
            if status.mode() == Ok(stm32wlxx_hal::subghz::StatusMode::StandbyHse) {
                // Radio returned to standby (timeout)
                warn!("  RX timeout");
                if let Some(ref mut sw) = self.rf_switch {
                    sw.set_off();
                }
                self.state = RadioState::Idle;
                return Ok(RxStatus::RxTimeout);
            }

            Timer::after_millis(10).await;
        }
    }

    async fn rx_continuous(&mut self, rx_buf: &mut [u8]) -> Result<(usize, RxQuality), Self::PhyError> {
        if !self.initialized || !self.rx_setup {
            return Err(RadioError::NotInitialized);
        }

        let subghz = self.subghz.as_mut().ok_or(RadioError::NotInitialized)?;

        // Start continuous RX
        subghz.set_rx(Timeout::DISABLED)
            .map_err(|_| RadioError::ReceptionFailed)?;

        self.state = RadioState::Receiving;

        // Wait for RX completion
        loop {
            let status = subghz.status()
                .map_err(|_| RadioError::ReceptionFailed)?;

            if status.cmd() == Ok(CmdStatus::Complete) {
                // Read packet status
                let pkt_status = subghz.lora_packet_status()
                    .map_err(|_| RadioError::ReceptionFailed)?;
                
                self.last_snr = pkt_status.snr_pkt().to_integer().clamp(-128, 127) as i8;
                self.last_rssi = pkt_status.rssi_pkt().to_integer();

                // Read payload
                let payload_len = rx_buf.len().min(255);
                subghz.read_buffer(0, &mut rx_buf[..payload_len])
                    .map_err(|_| RadioError::ReceptionFailed)?;

                // Return to standby
                subghz.set_standby(StandbyClk::Hse)
                    .map_err(|_| RadioError::ReceptionFailed)?;

                // Turn off RF switch after RX
                if let Some(ref mut sw) = self.rf_switch {
                    sw.set_off();
                }
                
                self.state = RadioState::Idle;
                let quality = RxQuality::new(self.last_rssi, self.last_snr);
                info!("  RX complete: {} bytes, SNR={}, RSSI={}", payload_len, self.last_snr, self.last_rssi);
                return Ok((payload_len, quality));
            }

            Timer::after_millis(10).await;
        }
    }

    async fn low_power(&mut self) -> Result<(), Self::PhyError> {
        if !self.initialized {
            return Ok(());
        }

        let subghz = self.subghz.as_mut().ok_or(RadioError::NotInitialized)?;

        // Put radio in sleep mode
        unsafe {
            subghz.set_sleep(SleepCfg::default())
                .map_err(|_| RadioError::ConfigurationFailed)?;
        }

        self.state = RadioState::Idle;
        Ok(())
    }
}

// Implement Timings trait for lorawan-device
impl Timings for SubGhzRadio {
    fn get_rx_window_lead_time_ms(&self) -> u32 {
        5 // RX window lead time in milliseconds
    }
}
