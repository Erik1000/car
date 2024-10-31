#![no_std]
#![no_main]

use bt_hci::controller::ExternalController;
use embassy_futures::join::join;
use esp_backtrace as _;

use esp_hal::{
    gpio::{GpioPin, Io},
    timer::{
        systimer::{SystemTimer, Target},
        timg::TimerGroup,
    },
};
use esp_wifi::ble::controller::asynch::BleConnector;
use key::{KeyListener, ENGINE_IN_PIN, IGNITION_IN_PIN, RADIO_IN_PIN};
use relay::RelayHandler;
extern crate alloc;

mod ble;
mod key;
mod relay;
mod schema;

#[esp_hal_embassy::main]
async fn main(spawner: embassy_executor::Spawner) {
    let peripherals = esp_hal::init(esp_hal::Config::default());
    let io = Io::new(peripherals.GPIO, peripherals.IO_MUX);
    let pins = io.pins;
    esp_alloc::heap_allocator!(72 * 1024);

    esp_println::logger::init_logger_from_env();

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let init = esp_wifi::init(
        esp_wifi::EspWifiInitFor::Ble,
        timg0.timer0,
        esp_hal::rng::Rng::new(peripherals.RNG),
        peripherals.RADIO_CLK,
    )
    .unwrap();

    let systimer = SystemTimer::new(peripherals.SYSTIMER).split::<Target>();
    esp_hal_embassy::init(systimer.alarm0);

    let bluetooth = peripherals.BT;
    let connector = BleConnector::new(&init, bluetooth);
    let controller: ExternalController<_, 20> = ExternalController::new(connector);

    let mut relay_handler = RelayHandler::new(pins.gpio10, pins.gpio20, pins.gpio7);
    spawner
        .spawn(key_task(pins.gpio1, pins.gpio3, pins.gpio4))
        .unwrap();

    join(ble::run(controller), relay_handler.listen()).await;
}

#[embassy_executor::task]
async fn key_task(
    radio: GpioPin<{ RADIO_IN_PIN }>,
    engine: GpioPin<{ ENGINE_IN_PIN }>,
    ignition: GpioPin<{ IGNITION_IN_PIN }>,
) {
    let mut key_listener = KeyListener::new(radio, engine, ignition);
    key_listener.listen().await;
}
