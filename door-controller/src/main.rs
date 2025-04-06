use std::sync::{mpsc, Arc};

use controller::{Controller, Operation};
use esp_idf_svc::{
    bt::BtDriver,
    eventloop::EspSystemEventLoop,
    hal::{gpio::PinDriver, prelude::Peripherals},
    nvs::{EspDefaultNvsPartition, EspNvs},
    ota::{EspOta, SlotState},
};

mod ble;
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
// 8 GPIO13

fn main() -> anyhow::Result<()> {
    // It is necessary to call this function once. Otherwise some patches to the runtime
    // implemented by esp-idf-sys might not link properly. See https://github.com/esp-rs/esp-idf-template/issues/71
    esp_idf_svc::sys::link_patches();

    // Bind the log crate to the ESP Logging facilities
    esp_idf_svc::log::EspLogger::initialize_default();

    let peripherals = Peripherals::take()?;
    let sys_loop = EspSystemEventLoop::take()?;
    let nvs_part = EspDefaultNvsPartition::take()?;

    let nvs = EspNvs::new(nvs_part.clone(), "controller", true)?;
    let enter_update = nvs.get_u8("enter_update")?.unwrap_or(0);

    if enter_update == 1 {
        let mut ota = EspOta::new()?;
        let running_slot = ota.get_running_slot()?;
        log::info!("Running slot: {running_slot:?}");
        if running_slot.state != SlotState::Valid {
            ota.mark_running_slot_valid()?;
        }
        drop(ota);

        updater::run_ota_update_mode(peripherals.modem, sys_loop, nvs_part)?;
    } else {
        let bt = BtDriver::new(peripherals.modem, Some(nvs_part.clone()))?;
        let pins = peripherals.pins;

        let door_open = PinDriver::output(pins.gpio32)?;
        let door_close = PinDriver::output(pins.gpio33)?;
        let door_disconnect = PinDriver::output(pins.gpio25)?;
        let window_left_up = PinDriver::output(pins.gpio26)?;
        let window_left_down = PinDriver::output(pins.gpio27)?;
        let window_right_up = PinDriver::output(pins.gpio14)?;
        let window_right_down = PinDriver::output(pins.gpio12)?;

        let (tx, rx) = mpsc::channel::<Operation>();
        let controller = Controller {
            rx,
            door_open,
            door_close,
            door_disconnect,
            window_left_up,
            window_left_down,
            window_right_up,
            window_right_down,
        };

        std::thread::spawn(move || controller.run());
        ble::start(tx, nvs, Arc::new(bt))?;
    }
    Ok(())
}
