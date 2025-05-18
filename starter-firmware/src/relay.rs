use core::convert::Infallible;

use embassy_futures::select::{select, Either};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use embassy_time::Timer;
use esp_hal::{
    gpio::{DriveMode, Level, Output, OutputConfig, Pull},
    peripherals::{GPIO10, GPIO20, GPIO21, GPIO7},
};
use log::{info, warn};

use crate::{
    key::SIGNAL_KEY_POSITION_CHANGE,
    schema::{EngineState, KeyPosition},
};

// GPIO pin numbers
const RADIO_OUT_PIN: u8 = 10;
const ENGINE_OUT_PIN: u8 = 21;
const ENGINE_CONSUMERS_OUT_PIN: u8 = 7;
const IGNITION_OUT_PIN: u8 = 20;

// FIXME: use RwLock when available in embassy-sync
// <https://github.com/embassy-rs/embassy/issues/1394>
pub static SIGNAL_ENGINE_STATE: Signal<CriticalSectionRawMutex, EngineState> = Signal::new();

pub static SIGNAL_BLE_STATE_CHANGE: Signal<CriticalSectionRawMutex, EngineState> = Signal::new();

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
enum RelayState {
    Powered,
    Unpowered,
}

// #[derive(Debug, Clone, Hash, PartialEq, Eq)]
// pub enum BleKeyPositionSync {
//     /// requested ble state is the same as the physical key position (e.g. Off)
//     Synced,
//     /// the physical is in any position except Off where is overwrites and other state set from ble
//     Overwrite,
//     /// The physical key position is set to Off and the state is set via Ble
//     Ble,
// }

impl Default for RelayState {
    fn default() -> Self {
        Self::Unpowered
    }
}

pub struct RelayHandler<'p> {
    relais: Relais<'p>,
    engine_running: bool,
    last_key_position: KeyPosition,
    current_state: EngineState,
    current_state_set_by_relay: bool,
}

impl<'p> RelayHandler<'p> {
    pub fn new(
        radio: GPIO10<'p>,
        engine: GPIO21<'p>,
        engine_consumers: GPIO7<'p>,
        ignition: GPIO20<'p>,
    ) -> Self {
        let config = OutputConfig::default()
            .with_pull(Pull::Down)
            .with_drive_mode(DriveMode::PushPull);
        let radio: Output<'p> = Output::new(radio, Level::Low, config);
        let engine: Output<'p> = Output::new(engine, Level::Low, config);
        let engine_consumers: Output<'p> = Output::new(engine_consumers, Level::Low, config);
        let ignition: Output<'p> = Output::new(ignition, Level::Low, config);

        Self {
            relais: Relais::<'p> {
                radio: radio.into(),
                engine: engine.into(),
                engine_consumers: engine_consumers.into(),
                ignition: ignition.into(),
            },
            engine_running: false,
            last_key_position: KeyPosition::Off,
            current_state: EngineState::Off,
            current_state_set_by_relay: false,
        }
    }

    /// Changes the relais to create the desired [`EngineState`]
    pub async fn set_state(&mut self, engine_state: EngineState, skip_cooldown: bool) {
        let relais = &mut self.relais;
        let cooldown = if skip_cooldown {
            Timer::after_secs(0)
        } else {
            Timer::after_secs(2)
        };

        match engine_state {
            EngineState::Off => {
                relais.ignition.unpower();
                relais.engine.unpower();
                relais.engine_consumers.unpower();
                relais.radio.unpower();
                cooldown.await;
                self.engine_running = false;
            }
            EngineState::Radio => {
                relais.ignition.unpower();
                relais.engine.unpower();
                relais.engine_consumers.unpower();

                relais.radio.power();
                cooldown.await;
                self.engine_running = false;
            }
            EngineState::Engine => {
                relais.ignition.unpower();

                relais.radio.power();
                relais.engine.power();
                relais.engine_consumers.power();
                cooldown.await;
                self.engine_running = false;
            }
            EngineState::Running => {
                // TODO: store running status in flash in order to recover on crash so that the engine cannot turn of while driving
                relais.ignition.unpower();
                relais.engine.unpower();
                relais.engine_consumers.unpower();
                relais.radio.unpower();
                cooldown.await;
                relais.radio.power();
                relais.engine.power();
                relais.ignition.power();
                // TODO: keep ignition powered until it is certain that the engine is running, currently no way to check this
                if !skip_cooldown {
                    Timer::after_secs(1).await;
                    relais.ignition.unpower();
                    relais.engine_consumers.power();
                }
                self.engine_running = true;
            }
        }
        info!("sending update");
        self.current_state = engine_state.clone();
        SIGNAL_ENGINE_STATE.signal(engine_state);
        info!("done!");
    }

    #[allow(unused)]
    pub fn state(&self) -> EngineState {
        if self.engine_running {
            return EngineState::Running;
        }

        if self.relais.engine.is_powered() {
            return EngineState::Engine;
        }

        if self.relais.radio.is_powered() {
            return EngineState::Radio;
        }

        EngineState::Off
    }

    pub async fn listen(&mut self) -> Infallible {
        loop {
            match select(
                SIGNAL_KEY_POSITION_CHANGE.wait(),
                SIGNAL_BLE_STATE_CHANGE.wait(),
            )
            .await
            {
                Either::First(key_position) => {
                    info!("relay got key position change {key_position:?}");
                    // ignition imply not running
                    if key_position == KeyPosition::Ignition {
                        self.engine_running = false;
                    }

                    // this ensures that if the relay already set the state to Engine or Running, the key will not unneccessary stop the engine while tacking back control
                    if key_position == KeyPosition::Radio
                        && self.current_state_set_by_relay
                        && matches!(
                            self.current_state,
                            EngineState::Running | EngineState::Engine
                        )
                    {
                        warn!("key position changed to {key_position:?} but will be ignored because engine state is set to {:?} by relay", self.current_state);
                    } else {
                        self.current_state_set_by_relay = false;
                        self.set_state(key_position.as_engine_state(), true).await;
                    }
                    self.last_key_position = key_position;
                }
                Either::Second(requested_engine_state) => {
                    info!("relay got ble change {requested_engine_state:?}");
                    if self.last_key_position != KeyPosition::Off {
                        warn!("Ble requested state {requested_engine_state:?} but it is overwritten by the physical key to {:?}", self.current_state);
                    } else {
                        self.current_state_set_by_relay = true;
                        self.set_state(requested_engine_state, false).await;
                    }
                }
            }
        }
    }
}

struct Relais<'d> {
    radio: Relay<'d, RADIO_OUT_PIN>,
    engine: Relay<'d, ENGINE_OUT_PIN>,
    engine_consumers: Relay<'d, ENGINE_CONSUMERS_OUT_PIN>,
    ignition: Relay<'d, IGNITION_OUT_PIN>,
}

pub struct Relay<'d, const GPIO: u8> {
    pin: Output<'d>,
}

impl<'d, const GPIO: u8> From<Output<'d>> for Relay<'d, GPIO> {
    fn from(pin: Output<'d>) -> Self {
        Self { pin }
    }
}

#[allow(unused)]
impl<const GPIO: u8> Relay<'_, GPIO> {
    fn state(&self) -> RelayState {
        match self.pin.is_set_high() {
            true => RelayState::Powered,
            false => RelayState::Unpowered,
        }
    }

    /// Returns the previous relay state
    fn set_state(&mut self, state: RelayState) -> RelayState {
        let old = self.state();
        match state {
            RelayState::Powered => self.pin.set_high(),
            RelayState::Unpowered => self.pin.set_low(),
        }
        old
    }

    pub fn unpower(&mut self) {
        self.pin.set_low();
    }

    pub fn power(&mut self) {
        self.pin.set_high();
    }

    pub fn is_unpowered(&self) -> bool {
        self.pin.is_set_low()
    }

    pub fn is_powered(&self) -> bool {
        self.pin.is_set_high()
    }
}
