use core::convert::Infallible;

use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use embassy_time::Timer;
use esp_hal::gpio::{GpioPin, Level, Output, OutputPin};
use log::info;

use crate::schema::EngineState;

// GPIO pin numbers
const RADIO_OUT_PIN: u8 = 10;
const ENGINE_OUT_PIN: u8 = 20;
const IGNITION_OUT_PIN: u8 = 7;

pub static SIGNAL_ENGINE_STATE: Signal<CriticalSectionRawMutex, EngineState> = Signal::new();

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
enum RelayState {
    Powered,
    Unpowered,
}

impl Default for RelayState {
    fn default() -> Self {
        Self::Unpowered
    }
}

pub struct RelayHandler<'p> {
    relais: Relais<'p>,
    engine_running: bool,
}

impl<'p> RelayHandler<'p> {
    pub fn new(
        radio: GpioPin<RADIO_OUT_PIN>,
        engine: GpioPin<ENGINE_OUT_PIN>,
        ignition: GpioPin<IGNITION_OUT_PIN>,
    ) -> Self {
        let radio = Output::new_typed(radio, Level::Low);
        let engine = Output::new_typed(engine, Level::Low);
        let ignition = Output::new_typed(ignition, Level::Low);

        Self {
            relais: Relais {
                radio: radio.into(),
                engine: engine.into(),
                ignition: ignition.into(),
            },
            engine_running: false,
        }
    }

    /// Changes the relais to create the desired [`EngineState`]
    pub async fn set_state(&mut self, engine_state: EngineState) {
        let relais = &mut self.relais;
        match engine_state {
            EngineState::Off => {
                relais.ignition.unpower();
                relais.engine.unpower();
                relais.radio.unpower();
                Timer::after_secs(5).await;
                self.engine_running = false;
            }
            EngineState::Radio => {
                relais.ignition.unpower();
                relais.engine.unpower();
                relais.radio.power();
                Timer::after_secs(5).await;
                self.engine_running = false;
            }
            EngineState::Engine => {
                relais.ignition.unpower();
                relais.radio.power();
                relais.engine.power();
                Timer::after_secs(5).await;
                self.engine_running = false;
            }
            EngineState::Running => {
                // TODO: store running status in flash in order to recover on crash so that the engine cannot turn of while driving
                relais.ignition.unpower();
                relais.engine.unpower();
                relais.radio.unpower();
                Timer::after_secs(5).await;
                relais.radio.power();
                relais.engine.power();
                relais.ignition.power();
                // TODO: keep ignition powered until it is certain that the engine is running, currently no way to check this
                Timer::after_secs(1).await;
                relais.ignition.unpower();
                self.engine_running = true;
            }
        }
    }

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
            let state = SIGNAL_ENGINE_STATE.wait().await;
            info!("Changing engine state to {state:?}");
            self.set_state(state).await;
        }
    }
}

struct Relais<'d> {
    radio: Relay<'d, RADIO_OUT_PIN>,
    engine: Relay<'d, ENGINE_OUT_PIN>,
    ignition: Relay<'d, IGNITION_OUT_PIN>,
}

pub struct Relay<'d, const GPIO: u8> {
    pin: Output<'d, GpioPin<GPIO>>,
}

impl<'d, const GPIO: u8> From<Output<'d, GpioPin<GPIO>>> for Relay<'d, GPIO> {
    fn from(pin: Output<'d, GpioPin<GPIO>>) -> Self {
        Self { pin }
    }
}

#[allow(unused)]
impl<'d, const GPIO: u8> Relay<'d, GPIO>
where
    GpioPin<GPIO>: OutputPin,
{
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
