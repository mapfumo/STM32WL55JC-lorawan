//! Minimal LoRaWAN join test based on working solution
//! This is a stripped-down version to test LoRaWAN functionality first
#![no_std]
#![no_main]

mod iv;

use defmt::info;
use embassy_executor::Spawner;
use embassy_stm32::gpio::{Level, Output, Pin, Speed};
use embassy_stm32::rng::{self, Rng};
use embassy_stm32::spi::Spi;
use embassy_stm32::time::Hertz;
use embassy_stm32::{bind_interrupts, peripherals};
use embassy_stm32::rcc::*;
use embassy_time::Delay;
use lora_phy::lorawan_radio::LorawanRadio;
use lora_phy::sx126x::{self, Stm32wl, Sx126x, TcxoCtrlVoltage};
use lora_phy::LoRa;
use embassy_time::Timer;
use lorawan_device::async_device::{region, Device, EmbassyTimer, JoinMode, JoinResponse};
use lorawan_device::region::{AU915, Subband};
use lorawan_device::{AppEui, AppKey, DevEui};
use {defmt_rtt as _, panic_probe as _};

use self::iv::{InterruptHandler, Stm32wlInterfaceVariant, SubghzSpiDevice};

// AU915 region configuration for WisGate Edge Lite 2
const MAX_TX_POWER: u8 = 14;

bind_interrupts!(struct Irqs{
    SUBGHZ_RADIO => InterruptHandler;
    RNG => rng::InterruptHandler<peripherals::RNG>;
});

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let mut config = embassy_stm32::Config::default();
    {
        config.rcc.hse = Some(Hse {
            freq: Hertz(32_000_000),
            mode: HseMode::Bypass,
            prescaler: HsePrescaler::DIV1,
        });
        config.rcc.sys = Sysclk::PLL1_R;
        config.rcc.pll = Some(Pll {
            source: PllSource::HSE,
            prediv: PllPreDiv::DIV2,
            mul: PllMul::MUL6,
            divp: None,
            divq: Some(PllQDiv::DIV2), // PLL1_Q clock (32 / 2 * 6 / 2), used for RNG
            divr: Some(PllRDiv::DIV2), // sysclk 48Mhz clock (32 / 2 * 6 / 2)
        });
    }
    let p = embassy_stm32::init(config);

    let ctrl1 = Output::new(p.PC4.degrade(), Level::Low, Speed::High);
    let ctrl2 = Output::new(p.PC5.degrade(), Level::Low, Speed::High);
    let ctrl3 = Output::new(p.PC3.degrade(), Level::High, Speed::High);

    let spi = Spi::new_subghz(p.SUBGHZSPI, p.DMA1_CH1, p.DMA1_CH2);
    let spi = SubghzSpiDevice(spi);
    let use_high_power_pa = true;
    let config = sx126x::Config {
        chip: Stm32wl { use_high_power_pa },
        tcxo_ctrl: Some(TcxoCtrlVoltage::Ctrl1V7),
        use_dcdc: true,
        rx_boost: false,
    };
    let iv = Stm32wlInterfaceVariant::new(Irqs, use_high_power_pa, Some(ctrl1), Some(ctrl2), Some(ctrl3)).unwrap();
    let lora = LoRa::new(Sx126x::new(spi, iv, config), true, Delay).await.unwrap();

    let radio: LorawanRadio<_, _, MAX_TX_POWER> = lora.into();
    let mut au915 = AU915::new();
    // Setting join bias to subband 1 to match gateway channels (915.2-916.6 MHz)
    // Gateway is configured for channels 0-7 (915.2, 915.4, 915.6, 915.8, 916.0, 916.2, 916.4, 916.6)
    au915.set_join_bias(Subband::_1);
    let region: region::Configuration = au915.into();
    let mut device: Device<_, _, _> = Device::new(region, radio, EmbassyTimer::new(), Rng::new(p.RNG, Irqs));

    defmt::info!("Joining LoRaWAN network");

    // Application "TOT" credentials from LoRaServer config
    let join_mode = JoinMode::OTAA {
        deveui: DevEui::from([0xAC, 0x1F, 0x09, 0xFF, 0xFE, 0x1B, 0xCE, 0x23]), // DevEui: AC1F09FFFE1BCE23
        appeui: AppEui::from([0xb1, 0x30, 0xa8, 0x64, 0xc5, 0x29, 0x53, 0x56]), // AppEui: b130a864c5295356
        appkey: AppKey::from([0xb7, 0x26, 0x73, 0x9b, 0x78, 0xec, 0x4b, 0x9e, 0x92, 0x34, 0xe5, 0xd3, 0x5e, 0xa9, 0x68, 0x1b]), // AppKey: b726739b78ec4b9e9234e5d35ea9681b
    };

    loop {
        let join_result = device.join(&join_mode).await;
        if let Ok(JoinResponse::JoinSuccess) = join_result {
            info!("LoRaWAN network joined successfully!");
            break;
        }
        info!("Join failed: {:?}. Retrying in 5 seconds...", join_result);
        Timer::after_secs(5).await;
    }
}
