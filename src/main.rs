#![no_std]
#![no_main]

use core::fmt::Write;

use constcat::concat;
use embassy_executor::Spawner;
use embassy_net::{
    Runner, StackResources,
    dns::DnsSocket,
    tcp::client::{TcpClient, TcpClientState},
};
use embassy_time::{Duration, Timer, with_timeout};

// Installing global alloc + panic/backtrace handlers by importing for side effects.
use esp_alloc as _;
use esp_backtrace as _;

use esp_hal::{
    analog::adc::{Adc, AdcConfig, Attenuation},
    clock::CpuClock,
    ram,
    rng::Rng,
    rtc_cntl::{Rtc, sleep::TimerWakeupSource, wakeup_cause},
    timer::timg::TimerGroup,
};
use esp_println::println;
use esp_radio::wifi::{ClientConfig, Config, ModeConfig, WifiDevice};
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
        let x = STATIC_CELL.uninit().write(($val));
        x
    }};
}

// Deep-sleep interval (1 hour) for low battery drain.
const WAKE_INTERVAL_SECS: u64 = 60 * 60;
// Calibrated ADC maximum after sensor wiring (80% of 12-bit full scale).
const ADC_MAX: u16 = 3276;
// Maximum wait time to get DHCP lease after Wi-Fi link is up.
const DHCP_TIMEOUT_SECS: u64 = 30;
// Maximum wait time to connect station to AP.
const WIFI_CONNECT_TIMEOUT_SECS: u64 = 30;
// Maximum wait time for one HTTP webhook request.
const REQUEST_TIMEOUT_SECS: u64 = 10;
// Retry count per wake cycle to avoid infinite battery drain.
const MAX_PUBLISH_RETRIES: u8 = 3;

const WIFI_SSID: &str = env!("WIFI_SSID");
const WIFI_PASSWORD: &str = env!("WIFI_PASSWORD");
const WEBHOOK_URL_BASE: &str = env!("HOME_ASSISTANT_WEBHOOK_URL_BASE");
const DEVICE_ID: &str = env!("DEVICE_ID");
const WEBHOOK_URL: &str = concat!(WEBHOOK_URL_BASE, "/moisture_sensor_", DEVICE_ID);

// Main async task entrypoint provided by esp-rtos + embassy integration.
#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    // Build-time config values (set via env vars before flashing).

    // Initialize chip peripherals and choose max CPU clock for Wi-Fi reliability.
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    // Log wake cause for deep-sleep wakes; EN reset is a hard reset and will show Undefined.
    println!("Wake source: {:?}", wakeup_cause());

    // Create global heap regions used by async networking + Wi-Fi internals.
    // Reclaimed RAM is memory made available after boot steps complete.
    esp_alloc::heap_allocator!(#[ram(reclaimed)] size: 64 * 1024);
    esp_alloc::heap_allocator!(size: 36 * 1024);

    let moisture_percent = {
        // Prepare ADC channel on GPIO15 for moisture probe reads.
        let mut adc_config = AdcConfig::new();
        let mut pin = adc_config.enable_pin(peripherals.GPIO15, Attenuation::_11dB);
        let mut adc = Adc::new(peripherals.ADC2, adc_config);

        // Take one sample per wake cycle.
        let raw_adc = nb::block!(adc.read_oneshot(&mut pin)).unwrap();
        let moisture_percent = moisture_percent_from_adc(raw_adc);
        println!("Device PIN15 read {raw_adc} ({moisture_percent}%)");
        moisture_percent
    };

    // Start preemptive scheduler used by esp-radio/Wi-Fi internals.
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);

    // Initialize radio subsystem once scheduler is running.
    let radio = mk_static!(esp_radio::Controller<'static>, esp_radio::init().unwrap());

    // Build Wi-Fi controller + station interface.
    let (mut controller, interfaces) =
        esp_radio::wifi::new(radio, peripherals.WIFI, Config::default()).unwrap();
    let wifi_interface = interfaces.sta;

    // Build deterministic random seed for network stack internal IDs/ports.
    let rng = Rng::new();
    let seed = (rng.random() as u64) << 32 | rng.random() as u64;
    // Build embassy TCP/IP stack over the Wi-Fi station network device.
    let (stack, runner) = embassy_net::new(
        wifi_interface,
        embassy_net::Config::dhcpv4(Default::default()),
        mk_static!(StackResources<3>, StackResources::<3>::new()),
        seed,
    );

    // Spawn network runner task (must stay alive while networking is active).
    spawner.spawn(net_task(runner)).ok();

    // Configure Wi-Fi as station client with your SSID/password.
    controller
        .set_config(&ModeConfig::Client(
            ClientConfig::default()
                .with_ssid(WIFI_SSID.into())
                .with_password(WIFI_PASSWORD.into()),
        ))
        .unwrap();
    // Start the Wi-Fi driver state machine.
    controller.start_async().await.unwrap();

    // Connect to access point, but abort this wake cycle on timeout.
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

    // Wait for DHCP IPv4 configuration, again with a strict timeout.
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

    // Build a small TCP client and DNS resolver bound to the network stack.
    let tcp_client = TcpClient::new(
        stack,
        mk_static!(
            TcpClientState<1, 1024, 1024>,
            TcpClientState::<1, 1024, 1024>::new()
        ),
    );
    let dns_client = DnsSocket::new(stack);

    // Build compact JSON payload without dynamic heap allocation.
    let mut payload: heapless::String<160> = heapless::String::new();
    write!(&mut payload, "{{\"value\":{moisture_percent}}}").ok();

    // Try sending webhook with bounded retries.
    let mut sent = false;
    for attempt in 1..=MAX_PUBLISH_RETRIES {
        // Construct HTTP client each attempt.
        let mut client = HttpClient::new(&tcp_client, &dns_client);
        // Start POST request to Home Assistant webhook URL.
        println!("{}", WEBHOOK_URL);
        match client.request(Method::POST, WEBHOOK_URL).await {
            Ok(builder) => {
                // Receive buffer for HTTP response headers/body.
                let mut rx_buf = [0u8; 1024];
                // Explicit headers keep protocol behavior predictable.
                let headers = [
                    ("Connection", "close"),
                    ("Content-Type", "application/json"),
                ];
                // Attach headers + payload body.
                let mut request = builder.headers(&headers).body(payload.as_bytes());

                // Bound each HTTP attempt so we always return to sleep.
                match with_timeout(
                    Duration::from_secs(REQUEST_TIMEOUT_SECS),
                    request.send(&mut rx_buf),
                )
                .await
                {
                    Ok(Ok(_)) => {
                        println!("Succeeded on attempt {attempt}");
                        sent = true;
                        break;
                    }
                    Ok(Err(err)) => {
                        println!("Failed on attempt {attempt}: {err:?}");
                    }
                    Err(_) => {
                        println!("Timed out on attempt {attempt}");
                    }
                }
            }
            Err(err) => {
                println!("Failed to build webhook request on attempt {attempt}: {err:?}");
            }
        }

        // Small retry gap to avoid hammering AP/HA.
        Timer::after(Duration::from_secs(2)).await;
    }

    // Log final outcome for serial debugging.
    if !sent {
        println!("Failed to publish after {MAX_PUBLISH_RETRIES} attempts");
    }

    // Return to deep sleep after this single-sample publish cycle.
    // Pressing the onboard EN button performs a full reset, which also triggers another publish cycle.
    sleep_for_interval(peripherals.LPWR);
}

// Put device into deep sleep with timer wake.
// Manual publishes are triggered by pressing EN, which resets the chip and restarts `main`.
fn sleep_for_interval(lpwr: esp_hal::peripherals::LPWR) -> ! {
    // RTC controller owns deep-sleep entry.
    let mut rtc = Rtc::new(lpwr);
    // Periodic wake source for hourly telemetry.
    let timer = TimerWakeupSource::new(core::time::Duration::from_secs(WAKE_INTERVAL_SECS));
    // Enter deep sleep; chip resets on wake and restarts from `main`.
    println!("Entering deep sleep for {WAKE_INTERVAL_SECS}s (or press EN to reset now)");
    rtc.sleep_deep(&[&timer]);
}

// Embassy background task that continuously services the network stack.
#[embassy_executor::task]
async fn net_task(mut runner: Runner<'static, WifiDevice<'static>>) {
    runner.run().await;
}

// Convert raw ADC sample to integer percentage.
pub fn moisture_percent_from_adc(raw_adc: u16) -> u8 {
    let clamped = raw_adc.min(ADC_MAX) as u32;
    ((clamped * 100) / ADC_MAX as u32) as u8
}
