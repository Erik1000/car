use std::{collections::BTreeSet, convert::Infallible, time::Duration};

use btleplug::{
    api::{
        Central, CentralEvent, Manager as _, Peripheral as _, ScanFilter,
        WriteType,
    },
    platform::{Adapter, Manager, Peripheral},
};
use color_eyre::eyre::eyre;
use futures_util::{Stream, StreamExt};
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

pub static STARTER: RwLock<Option<Peripheral>> = RwLock::const_new(None);
pub static DOOR_CONTROLLER: RwLock<Option<Peripheral>> =
    RwLock::const_new(None);
pub static ENGINE_STATUS: RwLock<EngineCommand> =
    RwLock::const_new(EngineCommand::Off);

pub async fn init(
    env: &JNIEnv<'_>,
) -> color_eyre::Result<(
    UnboundedSender<Command>,
    JoinHandle<Result<(), color_eyre::Report>>,
    JoinHandle<Result<Infallible, color_eyre::Report>>,
    JoinHandle<Result<Infallible, color_eyre::Report>>,
    JoinHandle<Result<(), color_eyre::Report>>,
)> {
    btleplug::platform::init(env)?;

    let manager = Manager::new().await?;

    // get the first (and usually only) ble adapter
    let adapter = manager
        .adapters()
        .await?
        .into_iter()
        .next()
        .ok_or(eyre!("No adapter found"))?;
    let events = adapter.events().await?;

    let (sender, receiver) = unbounded_channel::<Command>();

    let events = tokio::spawn(handle_events(events));

    let search_handle = tokio::spawn(async move {
        log_error("search failed", search(&adapter).await)
    });
    let listener_handle = tokio::spawn(async {
        log_error("ble sender failed", listen(receiver).await)
    });
    let update_handle = tokio::spawn(async {
        log_error(
            "Update engine state listener failed",
            update_engine_state().await,
        )
    });
    Ok((
        sender,
        search_handle,
        listener_handle,
        update_handle,
        events,
    ))
}

async fn handle_events(
    events: impl Stream<Item = CentralEvent>,
) -> color_eyre::Result<()> {
    let mut events = std::pin::pin!(events);
    info!("Listening to android ble events");
    while let Some(event) = events.next().await {
        match event {
            CentralEvent::DeviceConnected(_p_id) => {}
            CentralEvent::DeviceDisconnected(p_id) => {
                if STARTER
                    .read()
                    .await
                    .as_ref()
                    .filter(|p| p.id() == p_id)
                    .is_some()
                {
                    // automatically try to reconnect
                    // only makes sense for starter because door controller
                    // disconects after starter is no longer in engine state
                    try_reconnect_starter().await;
                }
            }
            // other events are useless on android
            _ => (),
        }
    }
    Ok(())
}
async fn listen(
    mut receiver: UnboundedReceiver<Command>,
) -> color_eyre::Result<Infallible> {
    info!("Listening for commands that should be sent over BLE");
    while let Some(command) = receiver.recv().await {
        info!("Sending command via BLE: {command:?}");
        match command {
            Command::DoorController(command) => {
                info!("Sending {command:?} to door controller handler");
                let _ = log_error(
                    "Door command handler failed",
                    handle_door_command(command).await,
                );
            }
            Command::Engine(command) => {
                info!("Sending {command:?} to engine handler");
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
    info!("Checking if door controller is connected");
    try_reconnect_door_controller().await;

    let needed_char = match command {
        DoorControllerCommand::Lock | DoorControllerCommand::Unlock => {
            schema::DOOR_LOCK_CHAR
        }
        DoorControllerCommand::WindowLeftUp
        | DoorControllerCommand::WindowLeftDown => {
            schema::DOOR_WINDOW_LEFT_CHAR
        }
        DoorControllerCommand::WindowRightUp
        | DoorControllerCommand::WindowRightDown => {
            schema::DOOR_WINDOW_RIGHT_CHAR
        }
    };
    let guard = DOOR_CONTROLLER.read().await;
    let door_controller = guard.as_ref().ok_or(eyre!("no door controller"))?;
    let char = door_controller.characteristics().iter().find(|c| c.uuid == needed_char).cloned().ok_or(eyre!("Door controller is missing characteristic for {command:?}: {needed_char}"))?;
    let command: u8 = match command {
        DoorControllerCommand::Lock
        | DoorControllerCommand::WindowLeftUp
        | DoorControllerCommand::WindowRightUp => 0,
        DoorControllerCommand::Unlock
        | DoorControllerCommand::WindowLeftDown
        | DoorControllerCommand::WindowRightDown => 1,
    };
    info!("Writing {command} to characteristic {}", char.uuid);
    door_controller
        .write(&char, &[command], WriteType::WithResponse)
        .await?;

    Ok(())
}

async fn handle_engine_command(
    command: EngineCommand,
) -> color_eyre::Result<()> {
    try_reconnect_starter().await;

    let guard = STARTER.read().await;
    let starter = guard.as_ref().ok_or(eyre!("starter not initalized"))?;

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
    Ok(())
}

async fn update_engine_state() -> color_eyre::Result<Infallible> {
    // give the scanner some time to find the starter and connect
    sleep(Duration::from_secs(10)).await;
    loop {
        if !try_reconnect_starter().await {
            warn!("Engine state updater could not connect, retrying...");
            sleep(Duration::from_secs(1)).await;
            continue;
        }
        let guard = STARTER.read().await;
        let starter = guard.as_ref().ok_or(eyre!("starter not initalized"))?;
        let _ = starter.discover_services().await;
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
        drop(guard);
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
    }
}
pub async fn search(adapter: &Adapter) -> color_eyre::Result<()> {
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
    let mut wait = 0;
    'scan: loop {
        if wait < 30 {
            wait += 3;
        }
        debug!("Searching...");
        // give some time to scan
        sleep(Duration::from_secs(wait)).await;
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
                    info!("Found starter with address {}", p.address());
                    {
                        let mut guard = STARTER.write().await;
                        if guard.as_ref().is_none() {
                            *guard = Some(p);
                        } else {
                            error!("BLE starter already initalized",);
                        }
                    }
                    found_starter = true;
                    continue 'peripherals;
                } else if service.uuid == DOOR_SERVICE_UUID
                    && !found_door_controller
                {
                    info!("Found door controller with address {}", p.address());
                    {
                        let mut guard = DOOR_CONTROLLER.write().await;
                        if guard.as_ref().is_none() {
                            *guard = Some(p);
                        } else {
                            error!("BLE door controller already initalized")
                        }
                    }
                    found_door_controller = true;
                    continue 'peripherals;
                }
            }
        }
    }
    Ok(())
}

pub async fn try_reconnect_starter() -> bool {
    {
        if let Some(starter) = STARTER.read().await.as_ref() {
            if starter.is_connected().await.unwrap_or(false) {
                return true;
            }
        }
    }
    let mut guard = STARTER.write().await;
    if let Some(old_starter) = guard.take() {
        let adapter = btleplug::global_adapter();
        // this only creates a new BluetoothGattDevice in android land
        // this is neede because in android, if a device disconnect, you have
        // to create a new BluetoothGattDevice from the mac address, because
        // the old device will always return not connected
        // dont ask me why it is that way
        let new_starter = match adapter.add(old_starter.address()) {
            Ok(new_starter) => new_starter,
            Err(e) => {
                error!("Failed to reconnect starter: {e}");
                return false;
            }
        };
        for i in 0..10 {
            if new_starter.connect().await.is_ok() {
                let _ = new_starter.discover_services().await;
                *guard = Some(new_starter);
                return true;
            } else {
                info!("Reconnect to starter failed ({i}), retry...");
                sleep(Duration::from_secs(1)).await;
            }
        }
    }
    false
}

pub async fn try_reconnect_door_controller() -> bool {
    {
        if let Some(door_controller) = DOOR_CONTROLLER.read().await.as_ref() {
            if door_controller.is_connected().await.unwrap_or(false) {
                return true;
            }
        }
    }
    let mut guard = DOOR_CONTROLLER.write().await;
    if let Some(old_door_controller) = guard.take() {
        let adapter = btleplug::global_adapter();
        // this only creates a new BluetoothGattDevice in android land
        // this is neede because in android, if a device disconnect, you have
        // to create a new BluetoothGattDevice from the mac address, because
        // the old device will always return not connected
        // dont ask me why it is that way
        let new_door_controller =
            match adapter.add(old_door_controller.address()) {
                Ok(new_door_controller) => new_door_controller,
                Err(e) => {
                    error!("Failed to reconnect starter: {e}");
                    return false;
                }
            };
        for i in 0..10 {
            if new_door_controller.connect().await.is_ok() {
                let _ = new_door_controller.discover_services().await;
                *guard = Some(new_door_controller);
                return true;
            } else {
                info!("Reconnect to starter failed ({i}), retry...");
                sleep(Duration::from_secs(1)).await;
            }
        }
    }
    false
}
