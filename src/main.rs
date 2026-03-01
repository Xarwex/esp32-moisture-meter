#![no_std]
#![no_main]

use esp_backtrace as _;
use esp_hal::{
    analog::adc::{Adc, AdcConfig, Attenuation},
    delay::Delay,
};
use esp_println::println;

esp_bootloader_esp_idf::esp_app_desc!();

#[esp_hal::main]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default());

    let mut adc_config = AdcConfig::new();
    let mut pin = adc_config.enable_pin(peripherals.GPIO15, Attenuation::_11dB);
    let mut adc = Adc::new(peripherals.ADC2, adc_config);

    let delay = Delay::new();
    loop {
        let pin15_value = nb::block!(adc.read_oneshot(&mut pin)).unwrap();
        let moisture_percent = moisture_percent_from_adc(pin15_value);
        println!("PIN15 read {pin15_value} ({moisture_percent}%)");
        delay.delay_millis(1000);
    }
}

pub const ESP32_ADC_MAX: u16 = 4095;

pub fn moisture_percent_from_adc(raw_adc: u16) -> u8 {
    let clamped = raw_adc.min(ESP32_ADC_MAX) as u32;
    ((clamped * 100) / ESP32_ADC_MAX as u32) as u8
}
