use std::{
    convert::Infallible,
    io::ErrorKind,
    sync::OnceLock,
    time::{Duration, SystemTime},
};

use color_eyre::eyre::eyre;
use jni::{
    objects::{AutoLocal, JClass, JObject, JString},
    JNIEnv,
};
use jose::{
    crypto::hmac::Hs256,
    format::{Compact, DecodeFormat},
    jwa::JsonWebSigningAlgorithm,
    jwk::{symmetric::OctetSequence, IntoJsonWebKey, JwkVerifier},
    jws::{IntoVerifier, Unverified},
    policy::{Checkable, StandardPolicy},
    JsonWebKey, Jwt,
};
use jose::{crypto::hmac::Key as HmacKey, jwa::Hmac};

use serde_json::json;
use tokio::{
    fs::File,
    io::{AsyncReadExt, AsyncWriteExt},
    sync::{
        mpsc::{UnboundedReceiver, UnboundedSender},
        Mutex,
    },
    task::JoinHandle,
    time::sleep,
};

use crate::{
    ble::{try_reconnect_door_controller, ENGINE_STATUS},
    log_error,
    schema::{Command, DoorControllerCommand, EngineCommand},
};

static SMS_SENDER: OnceLock<UnboundedSender<Sms>> = OnceLock::new();
static SMS_VERIFIER: Mutex<Option<JwkVerifier>> = Mutex::const_new(None);
static SMS_AUTHORIZED_PHONE_NUMBERS: OnceLock<Vec<String>> = OnceLock::new();
const SMS_VERIFIER_KEY_PATH: &str =
    "/data/data/com.erik_tesar.car.remote/sms_verifer_key.json";

#[derive(Debug, serde::Deserialize)]
struct CommandRepr {
    cmd: Command,
}

pub async fn init(
    ble_sender: UnboundedSender<Command>,
) -> color_eyre::Result<JoinHandle<Result<Infallible, color_eyre::Report>>> {
    let (sender, receiver) = tokio::sync::mpsc::unbounded_channel::<Sms>();
    SMS_SENDER.set(sender).ok();
    {
        match File::open(SMS_VERIFIER_KEY_PATH)
            .await
            .map_err(|e| e.kind())
        {
            Ok(mut file) => {
                let mut buf = vec![];
                file.read_to_end(&mut buf).await?;
                let key: JsonWebKey = serde_json::from_slice(&buf)?;
                let key =
                    key.check(StandardPolicy::default()).map_err(|(_, e)| e)?;
                let verifier: JwkVerifier = key.into_verifier(
                    JsonWebSigningAlgorithm::Hmac(Hmac::Hs256),
                )?;
                let mut sms_verifier = SMS_VERIFIER.lock().await;
                *sms_verifier = Some(verifier);
                info!("Loaded SMS verifier key from storage");
            }
            Err(ErrorKind::NotFound) => {
                info!("SMS verifier key not setup yet, file not found")
            }
            _ => {
                error!("Error reading key file...")
            }
        }
    }
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
            "hold engine",
            async move {
                let restore_state = ENGINE_STATUS.read().await.to_owned();
                let already_in_engine = matches!(restore_state, EngineCommand::Engine | EngineCommand::Ignition);

                info!("Enable engine for door controller");
                if !already_in_engine {
                        ble_sender
                            .send(Command::Engine(EngineCommand::Engine))?;
                }


                // if already in engine then we can already use it
                if try_reconnect_door_controller().await {
                    let hold_engine = Duration::from_secs(match door_command {
                        DoorControllerCommand::Lock
                        | DoorControllerCommand::Unlock => 3,
                        DoorControllerCommand::WindowLeftDown
                        | DoorControllerCommand::WindowLeftUp
                        | DoorControllerCommand::WindowRightDown
                        | DoorControllerCommand::WindowRightUp => 10,
                    });
                    ble_sender.send(Command::DoorController(door_command))?;
                    // hold the engine in state `engine` because the door controller got no power otherwise
                    sleep(hold_engine).await;
                    if !already_in_engine {
                        info!("Restoring Engine state to {restore_state:?}");
                        ble_sender.send(Command::Engine(restore_state))?;
                    }
                    Ok(())
                } else {
                    Err(eyre!("Door controller not connected, cannot perform {door_command:?}"))
                }
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
        let authorized =
            if let Some(authorized) = SMS_AUTHORIZED_PHONE_NUMBERS.get() {
                authorized.contains(&sms.number.trim().to_string())
            } else {
                warn!("No phone number found for Admin contact");
                false
            };
        if !authorized {
            warn!(
                "Ignoring SMS from unauthorized number: {}\nSMS: {}",
                sms.number, sms.message
            );
            continue;
        }
        let mut sms_verifier = SMS_VERIFIER.lock().await;
        match &mut *sms_verifier {
            Some(verifier) => {
                let encoded: Compact = match sms.message.trim().parse() {
                    Ok(encoded) => encoded,
                    Err(e) => {
                        warn!("Invalid signed SMS: {e}");
                        continue;
                    }
                };
                let unverified: Unverified<Jwt<CommandRepr>> =
                    match Jwt::decode(encoded) {
                        Ok(o) => o,
                        Err(e) => {
                            warn!("Failed to parse signed SMS: {e:#?}");
                            continue;
                        }
                    };
                match unverified.verify(verifier) {
                    Ok(jws) => {
                        drop(sms_verifier);
                        let exp = jws.payload().expiration.unwrap_or(0);
                        let exp =
                            SystemTime::UNIX_EPOCH + Duration::from_secs(exp);
                        if SystemTime::now() > exp {
                            error!("Signed SMS is expired");
                            continue;
                        }

                        info!(
                            "Verified command: {:?}",
                            jws.payload().additional.cmd
                        );
                        match jws.payload().additional.cmd.to_owned() {
                            Command::Engine(engine) => {
                                ble_sender.send(Command::Engine(engine))?
                            }
                            Command::DoorController(door) => {
                                enable_engine_for_door_controller(
                                    ble_sender.clone(),
                                    door,
                                )
                                .await;
                            }
                        }
                    }
                    Err(e) => {
                        error!("SMS signature validation failed: {e}")
                    }
                }
            }
            None => {
                if sms.message.trim().starts_with("setup:") {
                    if let Some((_, key)) = sms.message.split_once("setup:") {
                        let key = key.trim();
                        let key = json!({
                            "kty": "oct",
                            "k": key,

                        });
                        let key: OctetSequence =
                            match serde_json::from_value(key) {
                                Ok(key) => key,
                                Err(e) => {
                                    error!("Invalid setup key: {e}");
                                    continue;
                                }
                            };
                        let verifier: HmacKey<Hs256> = match key.into_verifier(
                            JsonWebSigningAlgorithm::Hmac(Hmac::Hs256),
                        ) {
                            Ok(verifier) => verifier,
                            Err(e) => {
                                error!(
                                    "Cannot convert setup key to HmacKey: {e}"
                                );
                                continue;
                            }
                        };
                        let jwk = verifier.into_jwk(Some(()))?;
                        let ser = serde_json::to_vec(&jwk)?;
                        let jwk = jwk
                            .check(StandardPolicy::default())
                            .map_err(|(_, e)| e)?;
                        *sms_verifier = Some(jwk.into_verifier(
                            JsonWebSigningAlgorithm::Hmac(Hmac::Hs256),
                        )?);

                        info!("Setup SMS verification key");
                        let mut file =
                            File::create(SMS_VERIFIER_KEY_PATH).await?;
                        file.write_all(&ser).await?;
                        info!("Stored SMS verification key");
                    } else {
                        error!("setup is missing key");
                        continue;
                    }
                } else {
                    error!(
                        "Setup needed to process SMS, ignored sms: `{}`",
                        sms.message
                    )
                }
            }
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

#[no_mangle]
pub extern "system" fn Java_com_erik_1tesar_car_remote_RustService_provideAuthorizedPhoneNumbers(
    env: JNIEnv,
    _this: JClass,
    numbers: JObject,
) {
    let numbers = env.get_list(numbers).expect("provideded");
    info!("Getting authorized phone numbers from contacts");
    // JList is not std Iterator :/
    let mut iter = match numbers.iter() {
        Ok(o) => o,
        Err(_) => {
            error!("Error creating iter for authorized phone numbers");
            return;
        }
    };
    let mut collected: Vec<String> = vec![];
    // latest version no longer implements iterator, so this is future proof
    #[allow(clippy::while_let_on_iterator)]
    while let Some(obj) = iter.next() {
        let obj: AutoLocal = env.auto_local(obj);
        if let Ok(string) = env.get_string(obj.as_obj().into()) {
            if let Ok(string) = string.to_str() {
                collected.push(string.to_string().split_whitespace().collect());
            }
        }
    }
    info!("Collected following phone numbers: {collected:?}");
    let _ = SMS_AUTHORIZED_PHONE_NUMBERS.set(collected);
}
