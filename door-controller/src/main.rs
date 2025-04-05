use std::sync::Arc;

use esp_idf_svc::{
    bt::BtDriver,
    eventloop::EspSystemEventLoop,
    hal::prelude::Peripherals,
    nvs::{EspDefaultNvsPartition, EspNvs},
    ota::{EspOta, SlotState},
};

mod controller;
mod updater;

// von links oben bei power connector nach unten relays
// auch hier zu entnehmen <https://devices.esphome.io/devices/ESP32E-Relay-X8>
// 1 GPIO32
// 2 GPIO33
// 3 GPIO25
// 4 GPIO26
// 5 GPIO27
// 6 GPIO14
// 7 GPIO12
// 8 GPIO13

fn main() -> anyhow::Result<()> {
    // It is necessary to call this function once. Otherwise some patches to the runtime
    // implemented by esp-idf-sys might not link properly. See https://github.com/esp-rs/esp-idf-template/issues/71
    esp_idf_svc::sys::link_patches();

    // Bind the log crate to the ESP Logging facilities
    esp_idf_svc::log::EspLogger::initialize_default();

    let peripherals = Peripherals::take()?;
    let sys_loop = EspSystemEventLoop::take()?;
    let nvs = EspDefaultNvsPartition::take()?;

    let enter_update = EspNvs::new(nvs.clone(), "controller", true)?;
    let enter_update = enter_update.get_u8("enter_update")?.unwrap_or(0);

    if enter_update == 1 {
        let mut ota = EspOta::new()?;
        let running_slot = ota.get_running_slot()?;
        log::info!("Running slot: {running_slot:?}");
        if running_slot.state != SlotState::Valid {
            ota.mark_running_slot_valid()?;
        }
        drop(ota);

        updater::run_ota_update_mode(peripherals.modem, sys_loop, nvs)?;
    } else {
        let bt = BtDriver::new(peripherals.modem, Some(nvs.clone()))?;
        controller::start(Arc::new(bt))?;
    }
    Ok(())
}
