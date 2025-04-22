use std::{convert::Infallible, sync::OnceLock, time::Duration};

use color_eyre::eyre::eyre;
use jni::{
    objects::{JClass, JString},
    JNIEnv,
};
use tokio::{
    sync::mpsc::{UnboundedReceiver, UnboundedSender},
    task::JoinHandle,
    time::sleep,
};

use crate::{
    ble::ENGINE_STATUS,
    log_error,
    schema::{Command, DoorControllerCommand, EngineCommand},
};

static SMS_SENDER: OnceLock<UnboundedSender<Sms>> = OnceLock::new();

pub async fn init(
    ble_sender: UnboundedSender<Command>,
) -> color_eyre::Result<JoinHandle<Result<Infallible, color_eyre::Report>>> {
    let (sender, receiver) = tokio::sync::mpsc::unbounded_channel::<Sms>();
    SMS_SENDER.set(sender).ok();
    Ok(tokio::spawn(async {
        log_error(
            "Sms receiver from java failed",
            listen(ble_sender, receiver).await,
        )
    }))
}

pub async fn enable_engine_for_door_controller(
    ble_sender: UnboundedSender<Command>,
    door_command: DoorControllerCommand,
) {
    tokio::spawn(async {
        log_error(
            "hold engine failed",
            async move {
                let restore_state = ENGINE_STATUS.read().await.to_owned();
                ble_sender.send(Command::Engine(EngineCommand::Engine))?;
                // give the esp in the door time to boot
                sleep(Duration::from_secs(2)).await;
                let hold_engine = Duration::from_secs(match door_command {
                    DoorControllerCommand::Lock
                    | DoorControllerCommand::Unlock => 1,
                    DoorControllerCommand::WindowLeftDown
                    | DoorControllerCommand::WindowLeftUp
                    | DoorControllerCommand::WindowRightDown
                    | DoorControllerCommand::WindowRightUp => 10,
                    DoorControllerCommand::OtaConfirm
                    | DoorControllerCommand::OtaEnter => {
                        return Err(eyre!(
                        "OTA update requires manual engine state set to engine"
                    ));
                    }
                });
                ble_sender.send(Command::DoorController(door_command))?;
                // hold the engine in state `engine` because the door controller got no power otherwise
                sleep(hold_engine).await;
                ble_sender.send(Command::Engine(restore_state))?;
                Result::<_, color_eyre::Report>::Ok(())
            }
            .await,
        )
    });
}
pub async fn listen(
    ble_sender: UnboundedSender<Command>,
    mut sms_receiver: UnboundedReceiver<Sms>,
) -> color_eyre::Result<Infallible> {
    while let Some(sms) = sms_receiver.recv().await {
        if sms.number == env!("AUTHORIZED_PHONE_NUMBER") {
            //env!("AUTHORIZED_PHONE_NUMBER") {
            match sms.message.as_str() {
                "off" => {
                    ble_sender.send(Command::Engine(EngineCommand::Off))?;
                }
                "radio" => {
                    ble_sender.send(Command::Engine(EngineCommand::Radio))?;
                }
                "engine" => {
                    ble_sender.send(Command::Engine(EngineCommand::Engine))?;
                }
                "ignition" => {
                    ble_sender
                        .send(Command::Engine(EngineCommand::Ignition))?;
                }
                "lock" => {
                    enable_engine_for_door_controller(
                        ble_sender.clone(),
                        DoorControllerCommand::Lock,
                    )
                    .await
                }
                "unlock" => {
                    enable_engine_for_door_controller(
                        ble_sender.clone(),
                        DoorControllerCommand::Unlock,
                    )
                    .await
                }
                "window_left_up" => {
                    enable_engine_for_door_controller(
                        ble_sender.clone(),
                        DoorControllerCommand::WindowLeftUp,
                    )
                    .await
                }
                "window_left_down" => {
                    enable_engine_for_door_controller(
                        ble_sender.clone(),
                        DoorControllerCommand::WindowLeftDown,
                    )
                    .await
                }
                "window_right_down" => {
                    enable_engine_for_door_controller(
                        ble_sender.clone(),
                        DoorControllerCommand::WindowRightDown,
                    )
                    .await
                }
                "window_right_up" => {
                    enable_engine_for_door_controller(
                        ble_sender.clone(),
                        DoorControllerCommand::WindowRightUp,
                    )
                    .await
                }
                "ota_enter" => {
                    ble_sender.send(Command::DoorController(
                        DoorControllerCommand::OtaEnter,
                    ))?;
                }
                "ota_confirm" => {
                    ble_sender.send(Command::DoorController(
                        DoorControllerCommand::OtaConfirm,
                    ))?;
                }
                _ => {
                    warn!("Received invalid command via sms: {}", sms.message)
                }
            }
        } else {
            warn!("Received sms from unauthorized number: {}", sms.number);
        }
    }
    Err(eyre!("Channel hung up"))
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Sms {
    number: String,
    message: String,
}

#[no_mangle]
pub extern "system" fn Java_com_erik_1tesar_car_remote_SmsBroadcastReceiver_recvSms(
    env: JNIEnv,
    _this: JClass,
    number: JString,
    sms_text: JString,
) {
    match SMS_SENDER.get() {
        Some(sender) => {
            let number = match env.get_string(number) {
                Ok(o) => o.into(),
                Err(e) => {
                    error!("Failed to get sms number from java: {e:#?}");
                    return;
                }
            };

            let message = match env.get_string(sms_text) {
                Ok(o) => o.into(),
                Err(e) => {
                    error!("Failed to get sms text from java: {e:#?}");
                    return;
                }
            };
            match sender.send(Sms { number, message }) {
                Ok(_) => {}
                Err(e) => {
                    error!("Failed to send sms text to receiver: {e}")
                }
            }
        }
        None => {
            warn!("Dropped sms, because sms sender is not initalized")
        }
    }
}
