#!/usr/bin/env python3
"""Generate Home Assistant moisture_meter_package.yaml from devices.toml."""

from __future__ import annotations

import re
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent
CONFIG_PATH = ROOT / "devices.toml"
OUTPUT_PATH = ROOT / "moisture_meter_package.yaml"

SLUG_RE = re.compile(r"^[a-z0-9_]+$")


def validate_slug(slug: str) -> None:
    if not SLUG_RE.match(slug):
        raise ValueError(
            f"Invalid device slug '{slug}'. Allowed characters: a-z, 0-9, underscore."
        )


def device_id_for(slug: str, prefix: str) -> str:
    return f"{prefix}{slug.replace('_', '-')}"


def webhook_id_for(slug: str, prefix: str) -> str:
    return f"{prefix}{slug.upper()}"


def generate(config: dict) -> str:
    defaults = config.get("defaults", {})
    slugs = defaults.get("device_names", [])
    device_id_prefix = defaults.get("device_id_prefix", "esp32-moisture-")
    webhook_id_prefix = defaults.get("webhook_id_prefix", "ESP32_MOISTURE_WEBHOOK_")
    notify_service = defaults.get("notify_service", "notify.notify")

    if not isinstance(slugs, list) or not slugs:
        raise ValueError("defaults.device_names must be a non-empty array.")

    for slug in slugs:
        if not isinstance(slug, str):
            raise ValueError("Each device slug in defaults.device_names must be a string.")
        validate_slug(slug)

    out: list[str] = []
    out.extend(
        [
            "# GENERATED FILE - DO NOT EDIT BY HAND.",
            "# Source of truth: home-assistant/devices.toml",
            "# Regenerate with: python home-assistant/generate_package.py",
            "",
            "input_number:",
        ]
    )

    for slug in slugs:
        out.extend(
            [
                f"  moisture_meter_{slug}_percent:",
                f"    name: Moisture meter {slug} percent",
                "    min: 0",
                "    max: 100",
                "    step: 1",
                "    mode: box",
            ]
        )

    out.extend(["", "input_text:"])
    for slug in slugs:
        out.extend(
            [
                f"  moisture_meter_{slug}_device_id:",
                f"    name: Moisture meter {slug} id",
                f"    initial: {device_id_for(slug, device_id_prefix)}",
                "    max: 64",
            ]
        )

    out.extend(["", "input_datetime:"])
    for slug in slugs:
        out.extend(
            [
                f"  moisture_meter_{slug}_last_webhook:",
                f"    name: Moisture meter {slug} last webhook",
                "    has_date: true",
                "    has_time: true",
                f"  moisture_meter_{slug}_last_alert:",
                f"    name: Moisture meter {slug} last alert",
                "    has_date: true",
                "    has_time: true",
            ]
        )

    out.extend(["", "input_boolean:"])
    for slug in slugs:
        out.extend(
            [
                f"  moisture_meter_{slug}_low_alert_active:",
                f"    name: Moisture meter {slug} low alert active",
            ]
        )

    out.extend(["", "template:", "  - sensor:"])
    for slug in slugs:
        out.extend(
            [
                f"      - name: Soil Moisture {slug}",
                f"        unique_id: soil_moisture_{slug}_percent",
                '        unit_of_measurement: "%"',
                "        state_class: measurement",
                "        icon: mdi:water-percent",
                f"        state: \"{{{{ states('input_number.moisture_meter_{slug}_percent') | float(0) }}}}\"",
                "        availability: >",
                f"          {{% set last = states('input_datetime.moisture_meter_{slug}_last_webhook') %}}",
                "          {% if last in ['unknown', 'unavailable', ''] %}",
                "            false",
                "          {% else %}",
                "            {{ (as_timestamp(now()) - as_timestamp(last)) < 10800 }}",
                "          {% endif %}",
                "        attributes:",
                f"          device_id: \"{{{{ states('input_text.moisture_meter_{slug}_device_id') }}}}\"",
                f"          last_webhook: \"{{{{ states('input_datetime.moisture_meter_{slug}_last_webhook') }}}}\"",
            ]
        )

    out.extend(["", "automation:"])
    for slug in slugs:
        webhook_id = webhook_id_for(slug, webhook_id_prefix)
        expected_device_id = device_id_for(slug, device_id_prefix)
        out.extend(
            [
                f"  - id: moisture_meter_{slug}_webhook_ingest",
                f"    alias: Moisture meter {slug} webhook ingest",
                "    mode: single",
                "    trigger:",
                "      - platform: webhook",
                f"        webhook_id: {webhook_id}",
                "        allowed_methods:",
                "          - POST",
                "        local_only: true",
                "    condition:",
                "      - condition: template",
                "        value_template: >",
                "          {{ trigger.json is mapping and (",
                f"             trigger.json.device_id == '{expected_device_id}' or",
                f"             trigger.json.device_id == '{webhook_id}'",
                "          ) }}",
                "    action:",
                "      - variables:",
                '          moisture_percent: "{{ trigger.json.moisture_percent | int(0) }}"',
                '          device_id: "{{ trigger.json.device_id }}"',
                "      - service: input_number.set_value",
                "        target:",
                f"          entity_id: input_number.moisture_meter_{slug}_percent",
                "        data:",
                '          value: "{{ [0, [moisture_percent, 100] | min] | max }}"',
                "      - service: input_text.set_value",
                "        target:",
                f"          entity_id: input_text.moisture_meter_{slug}_device_id",
                "        data:",
                '          value: "{{ device_id }}"',
                "      - service: input_datetime.set_datetime",
                "        target:",
                f"          entity_id: input_datetime.moisture_meter_{slug}_last_webhook",
                "        data:",
                '          datetime: "{{ now().strftime(\'%Y-%m-%d %H:%M:%S\') }}"',
                "",
                f"  - id: moisture_meter_{slug}_low_alert",
                f"    alias: Moisture meter {slug} low alert",
                "    mode: single",
                "    trigger:",
                "      - platform: numeric_state",
                f"        entity_id: sensor.soil_moisture_{slug}",
                "        below: 30",
                "    condition:",
                "      - condition: state",
                f"        entity_id: input_boolean.moisture_meter_{slug}_low_alert_active",
                '        state: "off"',
                "      - condition: template",
                "        value_template: >",
                f"          {{% set last = states('input_datetime.moisture_meter_{slug}_last_alert') %}}",
                "          {% if last in ['unknown', 'unavailable', ''] %}",
                "            true",
                "          {% else %}",
                "            {{ (as_timestamp(now()) - as_timestamp(last)) > 21600 }}",
                "          {% endif %}",
                "    action:",
                f"      - service: {notify_service}",
                "        data:",
                f"          title: Moisture meter {slug} alert",
                "          message: >-",
                f"            {slug} soil moisture is low ({{{{ states('sensor.soil_moisture_{slug}') }}}}%).",
                "      - service: input_boolean.turn_on",
                "        target:",
                f"          entity_id: input_boolean.moisture_meter_{slug}_low_alert_active",
                "      - service: input_datetime.set_datetime",
                "        target:",
                f"          entity_id: input_datetime.moisture_meter_{slug}_last_alert",
                "        data:",
                '          datetime: "{{ now().strftime(\'%Y-%m-%d %H:%M:%S\') }}"',
                "",
                f"  - id: moisture_meter_{slug}_low_alert_clear",
                f"    alias: Moisture meter {slug} low alert clear",
                "    mode: single",
                "    trigger:",
                "      - platform: numeric_state",
                f"        entity_id: sensor.soil_moisture_{slug}",
                "        above: 40",
                "    condition:",
                "      - condition: state",
                f"        entity_id: input_boolean.moisture_meter_{slug}_low_alert_active",
                '        state: "on"',
                "    action:",
                "      - service: input_boolean.turn_off",
                "        target:",
                f"          entity_id: input_boolean.moisture_meter_{slug}_low_alert_active",
                "",
            ]
        )

    return "\n".join(out).rstrip() + "\n"


def main() -> None:
    config = tomllib.loads(CONFIG_PATH.read_text(encoding="utf-8"))
    rendered = generate(config)
    OUTPUT_PATH.write_text(rendered, encoding="utf-8")
    print(f"Generated {OUTPUT_PATH}")


if __name__ == "__main__":
    main()
