#![no_std]
#![no_main]

use core::fmt::Write;

use constcat::concat;
use embassy_executor::Spawner;
use embassy_net::{
    DhcpConfig, Runner, StackResources,
    dns::DnsSocket,
    tcp::client::{TcpClient, TcpClientState},
};
use embassy_time::Timer;

// Installing global alloc + panic/backtrace handlers by importing for side effects.
use esp_alloc as _;
use esp_backtrace as _;

use esp_hal::{
    analog::adc::{Adc, AdcConfig, Attenuation},
    clock::CpuClock,
    rng::Rng,
    timer::timg::TimerGroup,
};
use esp_println::println;
use esp_radio::wifi::{ClientConfig, ModeConfig, WifiDevice};
use heapless::String;
// Lightweight HTTP client for embedded targets.
use reqwless::{
    client::HttpClient,
    request::{Method, RequestBuilder},
};

// Adds app metadata expected by the bootloader/ESP tooling.
esp_bootloader_esp_idf::esp_app_desc!();

// Helper for creating `'static` allocations without `unsafe` scattered in code.
// In embedded Rust we often need static storage because executors and drivers hold references forever.
macro_rules! mk_static {
    ($t:ty,$val:expr) => {{
        static STATIC_CELL: static_cell::StaticCell<$t> = static_cell::StaticCell::new();
        #[deny(unused_attributes)]
        let x = STATIC_CELL.uninit().write($val);
        x
    }};
}
// Deep-sleep interval between webhook updates.
const WAKE_INTERVAL_SECS: u64 = 60 * 60 * 3; // 3 hours
// Once we're connected we don't really disconnect, so we can spam a bit. This will only go wrong if
// the WiFI goes out.
const WIFI_RETRY_SECS: u64 = 10;
// Calibration points from measured readings on the previous scale:
// 10% was fully wet and 60% was fully dry.
const WET_ADC: u16 = 2970;
const DRY_ADC: u16 = 1320;

const BUFFER_SIZE: usize = 4096;

const WIFI_SSID: &str = env!("WIFI_SSID");
const WIFI_PASSWORD: &str = env!("WIFI_PASSWORD");
const WEBHOOK_URL_BASE: &str = env!("HOME_ASSISTANT_WEBHOOK_URL_BASE");
const DEVICE_ID: &str = env!("DEVICE_ID");
const WEBHOOK_URL: &str = concat!(WEBHOOK_URL_BASE, "/moisture_sensor_", DEVICE_ID);

// Main async task entrypoint provided by esp-rtos + embassy integration.
#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    println!("This device will post to {WEBHOOK_URL}");

    // Initialize chip peripherals and choose max CPU clock for Wi-Fi reliability.
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);

    // // Create global heap regions used by async networking + Wi-Fi internals.
    // // Reclaimed RAM is memory made available after boot steps complete.
    // //esp_alloc::heap_allocator!(#[ram(reclaimed)] size: 64 * 1024);
    esp_alloc::heap_allocator!(#[unsafe(link_section = ".dram2_uninit")] size: 98767);

    let rng = Rng::new();
    let radio_init = &*mk_static!(
        esp_radio::Controller<'static>,
        esp_radio::init().expect("Failed to initialize Wi-Fi/BLE controller")
    );

    println!("Initializing wifi controller");
    let (mut wifi_controller, interfaces) =
        esp_radio::wifi::new(radio_init, peripherals.WIFI, Default::default())
            .expect("Failed to initialize Wi-Fi controller");

    let net_seed = rng.random() as u64 | ((rng.random() as u64) << 32);

    let dhcp_config = DhcpConfig::default();
    let config = embassy_net::Config::dhcpv4(dhcp_config);

    // Init network stack
    let (stack, runner) = embassy_net::new(
        interfaces.sta,
        config,
        mk_static!(StackResources<3>, StackResources::<3>::new()),
        net_seed,
    );

    println!("Spawning net task");
    spawner.spawn(net_task(runner)).unwrap();

    let mut adc_config = AdcConfig::new();
    let mut pin = adc_config.enable_pin(peripherals.GPIO34, Attenuation::_11dB);
    let mut adc = Adc::new(peripherals.ADC1, adc_config);

    // println!("Spawning moisture reporter");
    // spawner
    //     .spawn(moisture_reporting(wifi_controller, stack, pin, adc))
    //     .unwrap();

    let mut body: String<64> = String::new();
    loop {
        if esp_radio::wifi::sta_state() != esp_radio::wifi::WifiStaState::Connected {
            println!("We're not connected to WiFi - will try to connect to {WIFI_SSID}");
            if !wifi_controller.is_started().unwrap_or(false) {
                println!("Controller not initialized - starting");
                let client_config = ModeConfig::Client(
                    ClientConfig::default()
                        .with_ssid(WIFI_SSID.into())
                        .with_password(WIFI_PASSWORD.into()),
                );
                wifi_controller.set_config(&client_config).unwrap();
                println!("Starting WiFi...");
                wifi_controller.start_async().await.unwrap();
                println!("WiFi started!");
            }

            println!("Connecting to {WIFI_SSID}...");
            if let Err(e) = wifi_controller.connect_async().await {
                println!(
                    "Couldn't connect to {WIFI_SSID}, because {e}, retrying in {WIFI_RETRY_SECS}s"
                );
                Timer::after_secs(WIFI_RETRY_SECS).await;
                continue;
            }
            println!("Connected to {WIFI_SSID}")
        }

        let dns = DnsSocket::new(stack);
        let tcp_state = TcpClientState::<1, BUFFER_SIZE, BUFFER_SIZE>::new();
        let tcp = TcpClient::new(stack, &tcp_state);

        let mut client = HttpClient::new(&tcp, &dns);
        let mut buffer = [0; BUFFER_SIZE];

        let moisture_percent = {
            let raw_adc = nb::block!(adc.read_oneshot(&mut pin)).unwrap();
            println!("Read raw adc {raw_adc}");
            moisture_percent_from_adc(raw_adc)
        };
        body.clear();
        write!(body, "{{\"value\": {moisture_percent}}}").unwrap();

        match client.request(Method::POST, WEBHOOK_URL).await {
            Ok(http_req) => {
                println!("About to send payload '{body}'");
                let mut http_req = http_req
                    .body(body.as_bytes())
                    .headers(&[("Content-Type", "application/json")]);
                let response = http_req.send(&mut buffer).await.unwrap();

                println!("Got response {:?}", response.status);
            }
            Err(e) => println!("Got error {e:?} when trying to construct the http request"),
        }

        Timer::after_secs(WAKE_INTERVAL_SECS).await;
    }
}

#[embassy_executor::task]
async fn net_task(mut runner: Runner<'static, WifiDevice<'static>>) {
    runner.run().await;
}

// Convert raw ADC sample to integer percentage.
pub fn moisture_percent_from_adc(raw_adc: u16) -> u8 {
    let clamped_adc = raw_adc.clamp(DRY_ADC, WET_ADC);
    let dry_adc = u32::from(DRY_ADC);
    let wet_adc = u32::from(WET_ADC);

    (((u32::from(clamped_adc) - dry_adc) * 100) / (wet_adc - dry_adc)) as u8
}
