#![no_std]
#![no_main]

use esp_backtrace as _;
use esp_hal::{
    analog::adc::{Adc, AdcConfig, Attenuation},
    clock::ClockControl,
    delay::Delay,
    gpio::Io,
    peripherals::Peripherals,
    prelude::*,
    system::SystemControl,
};
use esp_println::println;

#[entry]
fn main() -> ! {
    let peripherals = Peripherals::take();
    let system = SystemControl::new(peripherals.SYSTEM);
    let clocks = ClockControl::boot_defaults(system.clock_control).freeze();

    let io = Io::new(peripherals.GPIO, peripherals.IO_MUX);

    let mut adc_config = AdcConfig::new();

    let mut pin = adc_config.enable_pin(io.pins.gpio15, Attenuation::Attenuation0dB);

    let mut adc = Adc::new(peripherals.ADC2, adc_config);

    let delay = Delay::new(&clocks);
    loop {
        let pin15_value = nb::block!(adc.read_oneshot(&mut pin)).unwrap();
        println!("PIN read {pin15_value}");
        delay.delay_millis(1000u32);
    }
}
