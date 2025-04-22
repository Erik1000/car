use std::{
    collections::{BTreeSet, HashSet},
    convert::Infallible,
    sync::OnceLock,
    time::Duration,
};

use btleplug::{
    api::{Central, Manager as _, Peripheral as _, ScanFilter, WriteType},
    platform::{Manager, Peripheral},
};
use color_eyre::eyre::eyre;
use jni::JNIEnv;
use tokio::{
    sync::{
        mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender},
        RwLock,
    },
    task::JoinHandle,
    time::sleep,
};

use crate::{
    log_error,
    schema::{
        self, Command, DoorControllerCommand, EngineCommand, DOOR_SERVICE_UUID,
        ENGINE_SERVICE_UUID,
    },
};

pub static STARTER: OnceLock<Peripheral> = OnceLock::new();
pub static DOOR_CONTROLLER: OnceLock<Peripheral> = OnceLock::new();
pub static ENGINE_STATUS: RwLock<EngineCommand> =
    RwLock::const_new(EngineCommand::Off);

pub async fn init(
    env: &JNIEnv<'_>,
) -> color_eyre::Result<(
    UnboundedSender<Command>,
    JoinHandle<Result<(), color_eyre::Report>>,
    JoinHandle<Result<Infallible, color_eyre::Report>>,
    JoinHandle<Result<Infallible, color_eyre::Report>>,
)> {
    btleplug::platform::init(&env)?;

    let (sender, receiver) = unbounded_channel::<Command>();

    let search_handle =
        tokio::spawn(async { log_error("search failed", search().await) });
    let listener_handle = tokio::spawn(async {
        log_error("ble sender failed", listen(receiver).await)
    });
    let update_handle = tokio::spawn(async {
        log_error(
            "Update engine state listener failed",
            update_engine_state().await,
        )
    });
    Ok((sender, search_handle, listener_handle, update_handle))
}

async fn listen(
    mut receiver: UnboundedReceiver<Command>,
) -> color_eyre::Result<Infallible> {
    info!("Listening for commands that should be sent over BLE");
    while let Some(command) = receiver.recv().await {
        match command {
            Command::DoorController(command) => {
                let _ = log_error(
                    "Door command handler failed",
                    handle_door_command(command).await,
                );
            }
            Command::Engine(command) => {
                let _ = log_error(
                    "Engine command handler failed",
                    handle_engine_command(command).await,
                );
            }
        };
    }
    Err(eyre!("Channel closed"))
}

async fn handle_door_command(
    command: DoorControllerCommand,
) -> color_eyre::Result<()> {
    match DOOR_CONTROLLER.get() {
        Some(door_controller) => {
            if !door_controller.is_connected().await? {
                debug!(
                    "Door controller is not connected, trying to connect..."
                );
                match door_controller.connect().await {
                    Ok(_) => {
                        debug!("Successfully connected to door controller");
                    }
                    Err(e) => match e {
                        btleplug::Error::DeviceNotFound => {
                            return Err(eyre!("Cannot connect to door controller, ensure power is on (engine key position)"))
                        }
                        _ => return Err(e.into()),
                    },
                }

                let needed_char = match command {
                    DoorControllerCommand::OtaEnter
                    | DoorControllerCommand::OtaConfirm => {
                        schema::DOOR_OTA_CHAR
                    }
                    DoorControllerCommand::Lock
                    | DoorControllerCommand::Unlock => schema::DOOR_LOCK_CHAR,
                    DoorControllerCommand::WindowLeftUp
                    | DoorControllerCommand::WindowLeftDown => {
                        schema::DOOR_WINDOW_LEFT_CHAR
                    }
                    DoorControllerCommand::WindowRightUp
                    | DoorControllerCommand::WindowRightDown => {
                        schema::DOOR_WINDOW_RIGHT_CHAR
                    }
                };
                let char = door_controller.characteristics().iter().find(|c| c.uuid == needed_char).cloned().ok_or(eyre!("Door controller is missing characteristic for {command:?}: {needed_char}"))?;
                let command: u8 = match command {
                    DoorControllerCommand::Lock
                    | DoorControllerCommand::WindowLeftUp
                    | DoorControllerCommand::WindowRightUp => 0,
                    DoorControllerCommand::Unlock
                    | DoorControllerCommand::WindowLeftDown
                    | DoorControllerCommand::WindowRightDown
                    | DoorControllerCommand::OtaEnter => 1,
                    DoorControllerCommand::OtaConfirm => 2,
                };
                door_controller
                    .write(&char, &[command], WriteType::WithResponse)
                    .await?;
            }
        }
        None => {
            warn!("Door controller not found, cannot perform command: {command:?}")
        }
    }
    Ok(())
}

async fn handle_engine_command(
    command: EngineCommand,
) -> color_eyre::Result<()> {
    match STARTER.get() {
        Some(starter) => {
            if !starter.is_connected().await? {
                warn!("Starter not connected, trying to connect...");
                // early exit here because starter is expected to always be connected
                starter.connect().await?;
            }
            let char = starter
                .characteristics()
                .iter()
                .find(|c| c.uuid == schema::ENGINE_STATE_CHAR)
                .cloned()
                .ok_or(eyre!(
                    "Starter is missing characteristic for {command:?}: {}",
                    schema::ENGINE_STATE_CHAR
                ))?;
            let command: u8 = match command {
                EngineCommand::Off => 0,
                EngineCommand::Radio => 1,
                EngineCommand::Engine => 2,
                EngineCommand::Ignition => 3,
            };
            starter
                .write(&char, &[command], WriteType::WithResponse)
                .await?;
        }
        None => {
            error!("Starter not found, cannot perform command: {command:?}")
        }
    }
    Ok(())
}

async fn update_engine_state() -> color_eyre::Result<Infallible> {
    // give the scanner some time to find the starter and connect
    sleep(Duration::from_secs(3)).await;
    loop {
        if let Some(starter) = STARTER.get() {
            starter.connect().await?;
            starter.discover_services().await?;
            let char = starter
                .characteristics()
                .iter()
                .find(|c| c.uuid == schema::ENGINE_STATE_CHAR)
                .cloned()
                .ok_or(eyre!("starter does not have engine state char"))?;
            starter.subscribe(&char).await?;
            use futures_util::future::ready;
            use futures_util::StreamExt;
            let mut stream = starter
                .notifications()
                .await?
                .filter(|notficiation| ready(notficiation.uuid == char.uuid));
            debug!("Listening for engine state updates");
            while let Some(update) = stream.next().await {
                let val = match update
                    .value
                    .first()
                    .ok_or(eyre!("Invalid response format"))?
                {
                    0 => EngineCommand::Off,
                    1 => EngineCommand::Radio,
                    2 => EngineCommand::Engine,
                    3 => EngineCommand::Ignition,
                    _ => return Err(eyre!("Invalid response format")),
                };
                debug!("Updating engine state to {val:?}");
                let mut state = ENGINE_STATUS.write().await;
                *state = val;
            }
        } else {
            sleep(Duration::from_secs(1)).await;
        }
    }
}
pub async fn search() -> color_eyre::Result<()> {
    let manager = Manager::new().await?;

    // get the first (and usually only) ble adapter
    let adapter = manager
        .adapters()
        .await?
        .into_iter()
        .next()
        .ok_or(eyre!("No bluetooth adapter found"))?;
    adapter
        .start_scan(ScanFilter {
            services: vec![
                schema::DOOR_SERVICE_UUID,
                schema::ENGINE_SERVICE_UUID,
            ],
        })
        .await?;

    info!("Scanning for BLE devices");
    let mut found_starter = false;
    let mut found_door_controller = false;
    'scan: loop {
        debug!("Searching...");
        // give some time to scan
        sleep(Duration::from_secs(5)).await;
        let peripherals = adapter.peripherals().await?;
        // used to early exit the for each if both devices are found
        debug!("Total peripherals found: {}", peripherals.len());

        'peripherals: for p in peripherals {
            if found_starter && found_door_controller {
                info!("Found both BLE devices");
                adapter.stop_scan().await?;
                break 'scan;
            }
            if let Err(e) = p.connect().await {
                warn!("Failed to connect to device {}: {e:?}", p.address())
            }
            if let Err(e) = p.discover_services().await {
                warn!(
                    "Error discovering services on peripheral {}: {e}",
                    p.address()
                )
            } else {
                debug!(
                    "Discoverd services on {}: {:?}",
                    p.address(),
                    p.services()
                        .iter()
                        .map(|s| s.uuid)
                        .collect::<BTreeSet<uuid::Uuid>>()
                )
            }
            for service in &p.services() {
                if service.uuid == ENGINE_SERVICE_UUID && !found_starter {
                    error!("Found starter with address {}", p.address());
                    STARTER.set(p).map_err(|p| {
                        eyre!(
                            "BLE starter already initalized to {}",
                            p.address()
                        )
                    })?;
                    found_starter = true;
                    continue 'peripherals;
                } else if service.uuid == DOOR_SERVICE_UUID
                    && !found_door_controller
                {
                    info!("Found door controller with address {}", p.address());
                    DOOR_CONTROLLER.set(p).map_err(|p| {
                        eyre!(
                            "BLE door controller already initalized to {}",
                            p.address()
                        )
                    })?;
                    found_door_controller = true;
                    continue 'peripherals;
                }
            }
        }
    }
    Ok(())
}
