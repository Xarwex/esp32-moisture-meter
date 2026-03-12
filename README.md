# ESP32 Moisture Meter -> Home Assistant

This project reads moisture on ESP32, converts it on-device, sends `{device_id, moisture_percent}` to Home Assistant by webhook, then deep-sleeps.

## Scalable Home Assistant setup (add device names, no copy-paste hunks)

Home Assistant config is generated from `home-assistant/devices.toml`.

- Add/remove devices by editing one list: `defaults.device_names`
- Regenerate package YAML with:

```bash
python home-assistant/generate_package.py
```

Generated file:

- `home-assistant/moisture_meter_package.yaml` (**do not edit manually**)

### Naming rules used by generator

For each device slug (example: `device01`):

- `device_id`: `esp32-moisture-` + slug (with `_` converted to `-`)
  - `device01` -> `esp32-moisture-device01`
- `webhook_id`: `ESP32_MOISTURE_WEBHOOK_` + slug uppercased
  - `device01` -> `ESP32_MOISTURE_WEBHOOK_DEVICE01`

You can change prefixes in `devices.toml`.

## Single-device setup (device01)

### 1) Generate and install Home Assistant package

```bash
python home-assistant/generate_package.py
```

Copy generated package into HA packages directory (for example `/config/packages/moisture_meter.yaml`).

If packages are not enabled, add to `configuration.yaml`:

```yaml
homeassistant:
  packages: !include_dir_named packages
```

Reload automations/templates/helpers or restart HA.

### 2) Prepare firmware env file

```bash
cp .env.flash.example .env.flash
```

For default `device01` generated config, these must match:

- `DEVICE_ID="esp32-moisture-device01"`
- `HOME_ASSISTANT_WEBHOOK_URL="http://box.lan:8123/api/webhook/ESP32_MOISTURE_WEBHOOK_DEVICE01"`

`DEVICE_ID` is optional now; if omitted, firmware infers it from the trailing webhook path segment.
`HOME_ASSISTANT_HOST` is also optional; if omitted (or missing port), firmware derives host:port from webhook URL.

Then export and flash:

```bash
set -a; . ./.env.flash; set +a
cargo run --release
```

(`.cargo/config.toml` already sets `espflash flash --monitor` as runner.)

### 3) Verify

1. Serial monitor shows Wi-Fi + webhook success.
2. HA sensor `sensor.soil_moisture_device01` updates.
3. Press board **EN** to force an immediate publish cycle.

## Adding another device

1. Edit `home-assistant/devices.toml` and append a new slug to `device_names`.
2. Run `python home-assistant/generate_package.py`.
3. Reload HA package.
4. Flash that device with matching `DEVICE_ID` and `HOME_ASSISTANT_WEBHOOK_URL`.
