use uuid::Uuid;

pub const ENGINE_SERVICE_UUID: Uuid =
    Uuid::from_u128(0x0e353531515942a092ff38e9e49ab7d1);

/// Read, Write, Notify
pub const ENGINE_STATE_CHAR: Uuid =
    Uuid::from_u128(0x13d24b593d134ef798dbe174869078e0);

pub const DOOR_SERVICE_UUID: Uuid =
    Uuid::from_u128(0x5eb5b1175231409ea1cab7689f488473);

/// 0 lock close, 1 lock open
pub const DOOR_LOCK_CHAR: Uuid =
    Uuid::from_u128(0x446f5ef8e88940988444e82331c92339);
/// 1 enter ota, 2 set ota valid
pub const DOOR_OTA_CHAR: Uuid =
    Uuid::from_u128(0xe32a319fcfa44838aac359fde6058ee1);
/// 0 window up, 1 window down
pub const DOOR_WINDOW_LEFT_CHAR: Uuid =
    Uuid::from_u128(0xb163c9c8b1ac445a8232b7b462bf6b91);
/// 0 window up, 1 window down
pub const DOOR_WINDOW_RIGHT_CHAR: Uuid =
    Uuid::from_u128(0x8f738eeebbb74cce8b82726a56532bdc);

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Command {
    DoorController(DoorControllerCommand),
    Engine(EngineCommand),
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum DoorControllerCommand {
    OtaEnter,
    OtaConfirm,
    Lock,
    Unlock,
    WindowLeftUp,
    WindowLeftDown,
    WindowRightUp,
    WindowRightDown,
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone)]
pub enum EngineCommand {
    Off,
    Radio,
    Engine,
    Ignition,
}
