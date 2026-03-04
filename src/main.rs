#![no_std]
#![no_main]

use core::fmt::Write;

use embassy_executor::Spawner;
use embassy_net::{
    Runner, StackResources,
    dns::DnsSocket,
    tcp::client::{TcpClient, TcpClientState},
};
use embassy_time::{Duration, Timer, with_timeout};
use esp_alloc as _;
use esp_backtrace as _;
use esp_hal::{
    analog::adc::{Adc, AdcConfig, Attenuation},
    clock::CpuClock,
    ram,
    rng::Rng,
    rtc_cntl::{Rtc, sleep::TimerWakeupSource},
    timer::timg::TimerGroup,
};
use esp_println::println;
use esp_radio::wifi::{ClientConfig, Config, ModeConfig, WifiDevice};
use reqwless::{
    client::HttpClient,
    request::{Method, RequestBuilder},
};

esp_bootloader_esp_idf::esp_app_desc!();

// When nightly support is available, this can be replaced by static_cell::make_static.
macro_rules! mk_static {
    ($t:ty,$val:expr) => {{
        static STATIC_CELL: static_cell::StaticCell<$t> = static_cell::StaticCell::new();
        #[deny(unused_attributes)]
        let x = STATIC_CELL.uninit().write(($val));
        x
    }};
}

const WAKE_INTERVAL_SECS: u64 = 60 * 60;
const DHCP_TIMEOUT_SECS: u64 = 30;
const WIFI_CONNECT_TIMEOUT_SECS: u64 = 30;
const REQUEST_TIMEOUT_SECS: u64 = 10;
const MAX_PUBLISH_RETRIES: u8 = 3;

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    let wifi_ssid = option_env!("WIFI_SSID").unwrap_or("YOUR_WIFI_SSID");
    let wifi_password = option_env!("WIFI_PASSWORD").unwrap_or("YOUR_WIFI_PASSWORD");
    let webhook_url = option_env!("HOME_ASSISTANT_WEBHOOK_URL")
        .unwrap_or("http://box.lan:8123/api/webhook/CHANGE_ME");
    let webhook_host = option_env!("HOME_ASSISTANT_HOST").unwrap_or("box.lan:8123");

    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    esp_alloc::heap_allocator!(#[ram(reclaimed)] size: 64 * 1024);
    esp_alloc::heap_allocator!(size: 36 * 1024);

    let mut adc_config = AdcConfig::new();
    let mut pin = adc_config.enable_pin(peripherals.GPIO15, Attenuation::_11dB);
    let mut adc = Adc::new(peripherals.ADC2, adc_config);

    let raw_adc = nb::block!(adc.read_oneshot(&mut pin)).unwrap();
    let moisture_percent = moisture_percent_from_adc(raw_adc);
    println!("PIN15 read {raw_adc} ({moisture_percent}%)");

    if wifi_ssid == "YOUR_WIFI_SSID" || webhook_url.ends_with("/CHANGE_ME") {
        println!(
            "Set WIFI_SSID, WIFI_PASSWORD, and HOME_ASSISTANT_WEBHOOK_URL env vars before flashing."
        );
        sleep_for_interval(peripherals.LPWR);
    }

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);

    let radio = mk_static!(esp_radio::Controller<'static>, esp_radio::init().unwrap());

    let (mut controller, interfaces) =
        esp_radio::wifi::new(radio, peripherals.WIFI, Config::default()).unwrap();
    let wifi_interface = interfaces.sta;

    let rng = Rng::new();
    let seed = (rng.random() as u64) << 32 | rng.random() as u64;
    let (stack, runner) = embassy_net::new(
        wifi_interface,
        embassy_net::Config::dhcpv4(Default::default()),
        mk_static!(StackResources<3>, StackResources::<3>::new()),
        seed,
    );

    spawner.spawn(net_task(runner)).ok();

    controller
        .set_config(&ModeConfig::Client(
            ClientConfig::default()
                .with_ssid(wifi_ssid.into())
                .with_password(wifi_password.into()),
        ))
        .unwrap();
    controller.start_async().await.unwrap();

    if with_timeout(
        Duration::from_secs(WIFI_CONNECT_TIMEOUT_SECS),
        controller.connect_async(),
    )
    .await
    .is_err()
    {
        println!("Wi-Fi connect timed out; skipping webhook publish");
        sleep_for_interval(peripherals.LPWR);
    }

    if with_timeout(
        Duration::from_secs(DHCP_TIMEOUT_SECS),
        stack.wait_config_up(),
    )
    .await
    .is_err()
    {
        println!("DHCP timed out; skipping webhook publish");
        sleep_for_interval(peripherals.LPWR);
    }

    let tcp_client = TcpClient::new(
        stack,
        mk_static!(
            TcpClientState<1, 1024, 1024>,
            TcpClientState::<1, 1024, 1024>::new()
        ),
    );
    let dns_client = DnsSocket::new(stack);

    let mut payload: heapless::String<128> = heapless::String::new();
    write!(
        &mut payload,
        "{{\"raw_adc\":{raw_adc},\"moisture_percent\":{moisture_percent}}}"
    )
    .ok();

    let mut sent = false;
    for attempt in 1..=MAX_PUBLISH_RETRIES {
        let mut client = HttpClient::new(&tcp_client, &dns_client);
        match client.request(Method::POST, webhook_url).await {
            Ok(builder) => {
                let mut rx_buf = [0u8; 1024];
                let headers = [
                    ("Connection", "close"),
                    ("Content-Type", "application/json"),
                    ("Host", webhook_host),
                ];
                let mut request = builder.headers(&headers).body(payload.as_bytes());

                match with_timeout(
                    Duration::from_secs(REQUEST_TIMEOUT_SECS),
                    request.send(&mut rx_buf),
                )
                .await
                {
                    Ok(Ok(_)) => {
                        println!("Webhook publish succeeded on attempt {attempt}");
                        sent = true;
                        break;
                    }
                    Ok(Err(err)) => {
                        println!("Webhook publish failed on attempt {attempt}: {err:?}");
                    }
                    Err(_) => {
                        println!("Webhook publish timed out on attempt {attempt}");
                    }
                }
            }
            Err(err) => {
                println!("Failed to build webhook request on attempt {attempt}: {err:?}");
            }
        }

        Timer::after(Duration::from_secs(2)).await;
    }

    if !sent {
        println!("Failed to publish after {MAX_PUBLISH_RETRIES} attempts");
    }

    sleep_for_interval(peripherals.LPWR);
}

fn sleep_for_interval(lpwr: esp_hal::peripherals::LPWR) -> ! {
    let mut rtc = Rtc::new(lpwr);
    let timer = TimerWakeupSource::new(core::time::Duration::from_secs(WAKE_INTERVAL_SECS));
    println!("Entering deep sleep for {WAKE_INTERVAL_SECS}s");
    rtc.sleep_deep(&[&timer]);
}

#[embassy_executor::task]
async fn net_task(mut runner: Runner<'static, WifiDevice<'static>>) {
    runner.run().await;
}

pub const ESP32_ADC_MAX: u16 = 4095;

pub fn moisture_percent_from_adc(raw_adc: u16) -> u8 {
    let clamped = raw_adc.min(ESP32_ADC_MAX) as u32;
    ((clamped * 100) / ESP32_ADC_MAX as u32) as u8
}
