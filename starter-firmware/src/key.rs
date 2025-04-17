use embassy_futures::join::join3;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use embassy_time::{Duration, Timer};
use esp_hal::gpio::{GpioPin, Input, InputConfig, InputPin, Level, Pull};
use log::{debug, trace, warn};

use crate::schema::KeyPosition;

pub static SIGNAL_KEY_POSITION_CHANGE: Signal<CriticalSectionRawMutex, KeyPosition> = Signal::new();

pub const RADIO_IN_PIN: u8 = 0;
pub const ENGINE_IN_PIN: u8 = 3;
pub const IGNITION_IN_PIN: u8 = 6;

/// Checks and listens for the position of the physical key in the car next to the stearing wheel
pub struct KeyListener<'d> {
    radio: Debounced<'d, RADIO_IN_PIN>,
    // engine has two pins that are switched at the same time but we only listen for one because they are switched at the same time anyway
    engine: Debounced<'d, ENGINE_IN_PIN>,
    ignition: Debounced<'d, IGNITION_IN_PIN>,
    last_position: KeyPosition,
    off_overwrite_count: u8,
    last_signal: KeyPosition,
}

impl<'d> KeyListener<'d> {
    pub fn new(
        radio: GpioPin<'d, RADIO_IN_PIN>,
        engine: GpioPin<'d, ENGINE_IN_PIN>,
        ignition: GpioPin<'d, IGNITION_IN_PIN>,
    ) -> Self {
        Self {
            radio: radio.into(),
            engine: engine.into(),
            ignition: ignition.into(),
            last_position: KeyPosition::Off,
            off_overwrite_count: 0,
            last_signal: KeyPosition::Off,
        }
    }

    pub async fn listen(&mut self) {
        // listen for state change
        loop {
            let res = join3(
                self.radio.wait_for_stable_state(),
                self.engine.wait_for_stable_state(),
                self.ignition.wait_for_stable_state(),
            )
            .await;
            trace!("key: {res:?}");

            // FIXME: ensure key position does not turn off between engine and ignition
            let key_position = match res {
                (Level::Low, Level::Low, Level::Low) => KeyPosition::Off,
                (Level::High, Level::Low, Level::Low) => KeyPosition::Radio,
                (_, Level::High, Level::Low) => KeyPosition::Engine,
                (_, _, Level::High) => KeyPosition::Ignition,
            };

            use KeyPosition::*;
            // the state that should be logically possbile. for example, ignition to off position is not possible without first having engine and radio
            let mut sound_key_position = match (&self.last_position, &key_position) {
                // rotation from ignition to off
                //
                // the physical contact between engine state and ignition state has a small cap where there is no contact, this is filtered out here
                (Ignition, Engine | Off) => Engine,
                // assume key was turned fast enough to skip engine
                (Ignition | Engine, Radio) => Radio,
                (Radio, Off) => Off,
                // rotation from off to ignition
                //
                (Off, Radio) => Radio,
                (Radio, Engine) => Engine,
                // the physical contact between engine state and ignition state has a small cap where there is no contact, this is filtered out here
                // assume radio was skipped because of fast rotation
                (Engine | Off, Ignition) => Ignition,
                // assume engine was skipped because of fast rotation
                (Radio, Ignition) => Ignition,
                // --
                // assume key was turned fast enough to skip radio
                (Off, Engine) => Engine,
                (Engine, Off) => {
                    warn!("fixed position from engine -> off to engine");
                    self.off_overwrite_count += 1;
                    Engine
                }
                _ if self.last_position == key_position => key_position,
                _ => {
                    warn!(
                        "unsound key position from {:?} to {:?}",
                        self.last_position, key_position
                    );
                    key_position
                }
            };
            if self.off_overwrite_count > 3 && sound_key_position == Engine {
                sound_key_position = Off;
                self.off_overwrite_count = 0;
            }

            trace!("Key position is {sound_key_position:?}");
            self.last_position = sound_key_position.clone();
            if sound_key_position != self.last_signal {
                debug!("Sending key position change signal");
                self.last_signal = sound_key_position.clone();
                SIGNAL_KEY_POSITION_CHANGE.signal(sound_key_position);
            }
        }
    }
}

impl<'a, const P: u8> From<GpioPin<'a, P>> for Debounced<'a, P>
where
    GpioPin<'a, P>: InputPin,
{
    fn from(pin: GpioPin<'a, P>) -> Self {
        // uses some default values
        Self::new(pin, 20, Duration::from_millis(10))
    }
}

struct Debounced<'d, const P: u8> {
    pin: Input<'d>,
    previous_state: Level,
    debounce_count: u8,
    threshold: u8,
    interval: Duration,
}

impl<'d, const P: u8> Debounced<'d, P>
where
    GpioPin<'d, P>: InputPin,
{
    pub fn new(pin: GpioPin<'d, P>, threshold: u8, interval: Duration) -> Self {
        Debounced {
            pin: Input::new(pin, InputConfig::default().with_pull(Pull::Down)),
            previous_state: Level::Low,
            debounce_count: 0,
            threshold,
            interval,
        }
    }

    pub async fn wait_for_stable_state(&mut self) -> Level {
        loop {
            let current_state = self.pin.level();
            if current_state == self.previous_state {
                self.debounce_count += 1;
                if self.debounce_count >= self.threshold {
                    // state is stable
                    self.debounce_count = 0;
                    return self.previous_state;
                }
            } else {
                self.debounce_count = 0;
                self.previous_state = current_state;
            }
            Timer::after(self.interval).await;
        }
    }
}
