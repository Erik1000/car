use esp_idf_svc::{
    eventloop::EspSystemEventLoop,
    hal::prelude::Peripherals,
    nvs::EspDefaultNvsPartition,
    ota::{EspOta, SlotState},
};

mod updater;

fn main() -> anyhow::Result<()> {
    // It is necessary to call this function once. Otherwise some patches to the runtime
    // implemented by esp-idf-sys might not link properly. See https://github.com/esp-rs/esp-idf-template/issues/71
    esp_idf_svc::sys::link_patches();

    // Bind the log crate to the ESP Logging facilities
    esp_idf_svc::log::EspLogger::initialize_default();

    let peripherals = Peripherals::take()?;
    let sys_loop = EspSystemEventLoop::take()?;
    let nvs = EspDefaultNvsPartition::take()?;

    log::info!("Hello, world!");

    let mut ota = EspOta::new()?;
    let running_slot = ota.get_running_slot()?;
    log::info!("Running slot: {running_slot:?}");
    if running_slot.state != SlotState::Valid {
        ota.mark_running_slot_valid()?;
    }

    updater::run_ota_update_mode(peripherals.modem, sys_loop, nvs)?;

    Ok(())
}
