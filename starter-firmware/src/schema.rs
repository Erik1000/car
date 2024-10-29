use serde::{Deserialize, Serialize};

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

/// State of the engine
///
/// Represents the actual engine state of the car. Difference to [`KeyPosition`]
/// is that while a key is only in [`KeyPosition::Ignition`] while starting the
/// car and then returns to [`KeyPosition::Engine`] this enum will stay in
/// [`EngineState::Running`] as long as the engine is actual running
#[derive(Debug, Hash, PartialEq, Eq, Clone, Deserialize, Serialize, Copy)]
#[serde(rename_all = "lowercase")]
pub enum EngineState {
    Off,
    Radio,
    Engine,
    Running,
}
