#![no_std]
#![no_main]

use core::ops::Range;

use bt_hci::controller::ExternalController;
use embassy_embedded_hal::adapter::BlockingAsync;
use embassy_futures::join::join;
use esp_backtrace as _;

use esp_hal::{
    gpio::GpioPin,
    rng::Trng,
    timer::{systimer::SystemTimer, timg::TimerGroup},
};
use esp_storage::FlashStorage;
use esp_wifi::ble::controller::BleConnector;
use key::{KeyListener, ENGINE_IN_PIN, IGNITION_IN_PIN, RADIO_IN_PIN};
use relay::RelayHandler;
extern crate alloc;

mod ble;
mod key;
mod relay;
mod schema;

// creds,    data, nvs,     0x110000, 0x2000,
pub const MAP_FLASH_RANGE: Range<u32> = 0x110000..(0x110000 + 0x2000);

#[esp_hal_embassy::main]
async fn main(spawner: embassy_executor::Spawner) {
    let peripherals = esp_hal::init(esp_hal::Config::default());
    esp_alloc::heap_allocator!(size: 72 * 1024);
    esp_println::logger::init_logger_from_env();

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let trng = Trng::new(peripherals.RNG, peripherals.ADC1);
    let rng = trng.rng;
    let init = esp_wifi::init(timg0.timer0, rng, peripherals.RADIO_CLK).unwrap();

    let systimer = SystemTimer::new(peripherals.SYSTIMER);
    esp_hal_embassy::init(systimer.alarm0);

    let flash = FlashStorage::new();
    let flash = BlockingAsync::new(flash);

    let bluetooth = peripherals.BT;
    let connector = BleConnector::new(&init, bluetooth);
    let controller: ExternalController<_, 20> = ExternalController::new(connector);

    let mut relay_handler = RelayHandler::new(
        peripherals.GPIO10,
        peripherals.GPIO21,
        peripherals.GPIO7,
        peripherals.GPIO20,
    );
    spawner
        .spawn(key_task(
            peripherals.GPIO0,
            peripherals.GPIO3,
            peripherals.GPIO6,
        ))
        .unwrap();

    let (res, _) = join(ble::run(controller, trng, flash), relay_handler.listen()).await;
    if let Err(e) = res {
        log::error!("BLE returned with error: {e:?}")
    }
}

#[embassy_executor::task]
async fn key_task(
    radio: GpioPin<'static, { RADIO_IN_PIN }>,
    engine: GpioPin<'static, { ENGINE_IN_PIN }>,
    ignition: GpioPin<'static, { IGNITION_IN_PIN }>,
) {
    let mut key_listener = KeyListener::new(radio, engine, ignition);
    key_listener.listen().await;
}
