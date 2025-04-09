use serde::{Deserialize, Serialize};
use trouble_host::{
    prelude::{AsGatt, FromGatt},
    types::gatt_traits::FromGattError,
};

/// Position of key in lock
///
/// Represents the position in which the key would normally be as this is what is logically simulated by the relays
#[derive(Debug, Hash, PartialEq, Eq, Clone, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum KeyPosition {
    Off,
    Radio,
    Engine,
    Ignition,
}

impl KeyPosition {
    pub fn as_engine_state(&self) -> EngineState {
        match self {
            Self::Off => EngineState::Off,
            Self::Radio => EngineState::Radio,
            Self::Engine => EngineState::Engine,
            Self::Ignition => EngineState::Running,
        }
    }
}
/// State of the engine
///
/// Represents the actual engine state of the car. Difference to [`KeyPosition`]
/// is that while a key is only in [`KeyPosition::Ignition`] while starting the
/// car and then returns to [`KeyPosition::Engine`] this enum will stay in
/// [`EngineState::Running`] as long as the engine is actual running
#[derive(Debug, Hash, PartialEq, Eq, Clone, Deserialize, Serialize, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
#[repr(u8)]
pub enum EngineState {
    Off = 0,
    Radio = 1,
    Engine = 2,
    Running = 3,
}

impl Default for EngineState {
    fn default() -> Self {
        Self::Off
    }
}

impl AsGatt for EngineState {
    const MAX_SIZE: usize = 1;
    const MIN_SIZE: usize = 1;
    fn as_gatt(&self) -> &'static [u8] {
        // this works because it is static but if done any other way the
        // compiler is too dumb to figure out the values are static
        match self {
            Self::Off => &[0],
            Self::Radio => &[1],
            Self::Engine => &[2],
            Self::Running => &[3],
        }
    }
}

impl FromGatt for EngineState {
    fn from_gatt(data: &[u8]) -> Result<Self, FromGattError> {
        Ok(match data.first().ok_or(FromGattError::InvalidLength)? {
            0 => Self::Off,
            1 => Self::Radio,
            2 => Self::Engine,
            3 => Self::Running,
            _ => return Err(FromGattError::InvalidCharacter),
        })
    }
}
