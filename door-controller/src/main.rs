#![no_std]
#![no_main]
#![feature(impl_trait_in_assoc_type)]
extern crate alloc;

use core::ops::Range;

use bt_hci::controller::ExternalController;
use controller::{Controller, Operation};
use embassy_embedded_hal::adapter::BlockingAsync;
use embassy_futures::join::join;
use embassy_sync::{
    blocking_mutex::raw::NoopRawMutex,
    channel::{self, Channel},
    once_lock::OnceLock,
};
use esp_backtrace as _;
use esp_hal::{
    gpio::{DriveMode, Output, OutputConfig, Pull},
    rng::Trng,
    timer::timg::TimerGroup,
};
use esp_storage::FlashStorage;
use esp_wifi::ble::controller::BleConnector;
use log::debug;

mod ble;
mod controller;
mod schema;

// von links oben bei power connector nach unten relays
// auch hier zu entnehmen <https://devices.esphome.io/devices/ESP32E-Relay-X8>
// 1 GPIO32
// 2 GPIO33
// 3 GPIO25
// 4 GPIO26
// 5 GPIO27
// 6 GPIO14
// 8 GPIO13

// creds,    data, nvs,     0x110000, 0x2000,
pub const MAP_FLASH_RANGE: Range<u32> = 0x110000..(0x110000 + 0x2000);

pub static CONTROLLER_CHANNEL: OnceLock<Channel<NoopRawMutex, Operation, 10>> = OnceLock::new();
#[esp_hal_embassy::main]
async fn main(_spawner: embassy_executor::Spawner) {
    esp_println::logger::init_logger_from_env();
    let peripherals = esp_hal::init(esp_hal::Config::default());
    debug!("Peripherals initalized");
    // FIXME: I think in debug the heap is too big and leaks into the stack or something
    // also there is an overflow in esp-hal/src/timer/timg.rs:589:5 which is
    // only catched in debug because of overflow checks but it works in release
    // perhaps this is a bug in esp-hal and they should be using wrapping methods?
    // release size: 482,416, debug size: 565,344 bytes
    // stack size of 72 was in the template but then debug crashes
    // 50 seems to work fine in both without allocation errors
    esp_alloc::heap_allocator!(size: 50 * 1024);
    debug!("Heap initalizied");

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let timg1 = TimerGroup::new(peripherals.TIMG1);
    let trng = Trng::new(peripherals.RNG, peripherals.ADC1);
    let rng = trng.rng;
    let init = esp_wifi::init(timg1.timer1, rng, peripherals.RADIO_CLK).unwrap();

    esp_hal_embassy::init(timg0.timer0);

    let flash = FlashStorage::new();
    let flash = BlockingAsync::new(flash);

    let bluetooth = peripherals.BT;
    let connector = BleConnector::new(&init, bluetooth);
    let ble_controller: ExternalController<_, 20> = ExternalController::new(connector);

    let config = OutputConfig::default()
        .with_pull(Pull::None)
        .with_drive_mode(DriveMode::PushPull);
    let door_open = Output::new(peripherals.GPIO32, esp_hal::gpio::Level::Low, config);
    let door_close = Output::new(peripherals.GPIO33, esp_hal::gpio::Level::Low, config);
    let door_disconnect = Output::new(peripherals.GPIO25, esp_hal::gpio::Level::Low, config);
    let window_left_up = Output::new(peripherals.GPIO26, esp_hal::gpio::Level::Low, config);
    let window_left_down = Output::new(peripherals.GPIO27, esp_hal::gpio::Level::Low, config);
    let window_right_up = Output::new(peripherals.GPIO14, esp_hal::gpio::Level::Low, config);
    let window_right_down = Output::new(peripherals.GPIO12, esp_hal::gpio::Level::Low, config);

    let channel =
        CONTROLLER_CHANNEL.get_or_init(channel::Channel::<NoopRawMutex, Operation, 10>::new);
    let rx = channel.receiver();
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

    let (_door, ble) = join(controller.run(), ble::run(ble_controller, trng, flash)).await;
    match ble {
        Ok(()) => log::info!("BLE returned with Ok"),
        Err(e) => log::error!("BLE returned with error: {e:#?}"),
    }
}
