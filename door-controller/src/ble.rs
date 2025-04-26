use core::{
    fmt::{Debug, Display},
    future::Future,
};

use alloc::borrow::ToOwned;
use embassy_embedded_hal::adapter::BlockingAsync;
use embassy_futures::join::join;
use embedded_io::Write;
use esp_hal::rng::Trng;
use esp_storage::FlashStorage;
use log::{debug, error, info, warn};
use sequential_storage::{
    cache::NoCache,
    map::{fetch_item, store_item, Key, SerializationError, Value},
};
use trouble_host::{prelude::*, BondInformation, IdentityResolvingKey, LongTermKey};

use crate::{
    schema::{Lock, WindowLeft, WindowRight},
    CONTROLLER_CHANNEL, MAP_FLASH_RANGE,
};

/// Max number of connections
const CONNECTIONS_MAX: usize = 1;

/// Max number of L2CAP channels.
const L2CAP_CHANNELS_MAX: usize = 2; // Signal + att

//const MAX_ATTRIBUTES: usize = 10;

pub const ADDRESS: [u8; 6] = [0xB7, 0x98, 0x49, 0x4E, 0x0D, 0x17];

// if too long it will leak into the advertisement packets
const BLE_DEVICE_NAME: &str = "DCtrl";

// Service UUID for Door Controller
pub const DOOR_SERVICE_UUID: u128 = 0x5eb5b1175231409ea1cab7689f488473;

// Used to initiate OTA update boot
// pub const OTA_CHAR_UUID: u128 = 0xe32a319fcfa44838aac359fde6058ee1;

// relay 1 and 2 and 3
// 1 GPIO32
// 2 GPIO33
// 3 GPIO25
// Recv only
pub const LOCK_CHAR_UUID: u128 = 0x446f5ef8e88940988444e82331c92339;

// Relay 4 and 5
// 4 GPIO26
// 5 GPIO27
// Recv only
pub const WINDOW_LEFT_CHAR_UUID: u128 = 0xb163c9c8b1ac445a8232b7b462bf6b91;

// Relay 6 and 7
// 6 GPIO14
// 7 GPIO12
// Recv only
pub const WINDOW_RIGHT_CHAR_UUID: u128 = 0x8f738eeebbb74cce8b82726a56532bdc;

#[gatt_service(uuid = DOOR_SERVICE_UUID)]
struct DoorControllerService {
    #[characteristic(uuid = LOCK_CHAR_UUID, write)]
    lock: Lock,
    #[characteristic(uuid = WINDOW_LEFT_CHAR_UUID, write)]
    window_left: WindowLeft,
    #[characteristic(uuid = WINDOW_RIGHT_CHAR_UUID, write)]
    window_right: WindowRight,
}

#[gatt_server]
struct Server<'a> {
    door_controller: DoorControllerService,
}

pub async fn run<C: Controller>(
    controller: C,
    mut rng: Trng<'_>,
    mut flash: BlockingAsync<FlashStorage>,
) -> Result<(), Error> {
    let address = Address::random(ADDRESS);

    let mut resources: HostResources<DefaultPacketPool, CONNECTIONS_MAX, L2CAP_CHANNELS_MAX> =
        HostResources::new();
    let stack = trouble_host::new(controller, &mut resources)
        .set_random_address(address)
        .set_random_generator_seed(&mut rng);

    let Host {
        mut peripheral,
        runner,
        ..
    } = stack.build();

    info!("Loading stored bond info if available");
    if let Some(bond_info) = load_bond_info(&mut flash).await {
        stack.add_bond_information(bond_info)?;
    }

    let server = Server::new_with_config(GapConfig::Peripheral(PeripheralConfig {
        name: BLE_DEVICE_NAME,
        appearance: &trouble_host::prelude::appearance::control_device::GENERIC_CONTROL_DEVICE,
    }))
    .map_err(|_| Error::Other)?;

    let _ = join(log_error("ble_task", ble_task(runner)), async {
        loop {
            match advertise_task(&mut peripheral, &server).await {
                Ok(conn) => {
                    if let Err(e) = gatt_task(&server, &conn, &stack, &mut flash).await {
                        log::error!("Gatt task error: {e:#?}")
                    }
                }
                Err(e) => {
                    log::error!("{e:?}");
                }
            }
        }
    })
    .await;
    Ok(())
}

async fn log_error<T, E>(name: impl Display, fut: impl Future<Output = Result<T, E>>)
where
    E: Debug,
{
    match fut.await {
        Ok(_) => info!("{name} has returned with no error"),
        Err(e) => error!("{name} has returend with an error: {e:#?}"),
    }
}
async fn ble_task<C: Controller>(
    mut runner: Runner<'_, C, DefaultPacketPool>,
) -> Result<(), BleHostError<C::Error>> {
    runner.run().await?;
    Ok(())
}

async fn gatt_task(
    server: &Server<'_>,
    conn: &GattConnection<'_, '_, DefaultPacketPool>,
    stack: &Stack<'_, impl Controller, DefaultPacketPool>,
    flash: &mut BlockingAsync<FlashStorage>,
) -> Result<(), Error> {
    info!("gatt task running");
    let lock_state = &server.door_controller.lock;
    let window_left_state = &server.door_controller.window_left;
    let window_right_state = &server.door_controller.window_right;
    let controller_sender = CONTROLLER_CHANNEL.get().await.sender();
    loop {
        match conn.next().await {
            GattConnectionEvent::Gatt { event } => match event? {
                GattEvent::Read(read) => {
                    if conn.raw().encrypted() {
                        let forbidden = [
                            lock_state.handle,
                            window_left_state.handle,
                            window_right_state.handle,
                        ];
                        if forbidden.contains(&read.handle()) {
                            read.reject(AttErrorCode::WRITE_REQUEST_REJECTED)?
                                .send()
                                .await;
                            continue;
                        }
                    } else {
                        read.reject(AttErrorCode::INSUFFICIENT_ENCRYPTION)?
                            .send()
                            .await;
                    }
                }
                GattEvent::Write(event) => {
                    if conn.raw().encrypted() {
                        if event.handle() == lock_state.handle {
                            match Lock::from_gatt(event.data()) {
                                Ok(val) => controller_sender.send(val.into()).await,
                                Err(_) => {
                                    log::error!(
                                        "Rejected write event because of invalid value: {:?}",
                                        event.data()
                                    );
                                    event.reject(AttErrorCode::VALUE_NOT_ALLOWED)?.send().await;
                                    continue;
                                }
                            };
                        } else if event.handle() == window_left_state.handle {
                            match WindowLeft::from_gatt(event.data()) {
                                Ok(val) => controller_sender.send(val.into()).await,
                                Err(_) => {
                                    log::error!(
                                        "Rejected write event because of invalid value: {:?}",
                                        event.data()
                                    );
                                    event.reject(AttErrorCode::VALUE_NOT_ALLOWED)?.send().await;
                                    continue;
                                }
                            }
                        } else if event.handle() == window_right_state.handle {
                            match WindowRight::from_gatt(event.data()) {
                                Ok(val) => controller_sender.send(val.into()).await,
                                Err(_) => {
                                    log::error!(
                                        "Rejected write event because of invalid value: {:?}",
                                        event.data()
                                    );
                                    event.reject(AttErrorCode::VALUE_NOT_ALLOWED)?.send().await;
                                    continue;
                                }
                            }
                        } else {
                            log::warn!("Write to known handle: {}", event.handle());
                        }
                    } else {
                        event
                            .reject(AttErrorCode::INSUFFICIENT_ENCRYPTION)?
                            .send()
                            .await;
                    }
                }
            },
            GattConnectionEvent::Disconnected { reason } => {
                log::warn!("Disconnected: {reason:?}");
                break;
            }
            GattConnectionEvent::Bonded { bond_info } => {
                log::info!("Bonding with new device: {bond_info:x?}");
                if load_bond_info(flash).await.is_none() {
                    stack.add_bond_information(bond_info.clone())?;
                    if bond_info.identity.irk.is_some() {
                        store_bond_info(flash, bond_info).await;
                    }
                    log::info!("Stored bond");
                } else {
                    warn!(
                        "Ignored bond from {:x?} since already bonded",
                        bond_info.identity.bd_addr
                    );
                    if let Err(e) = stack.remove_bond_information(bond_info.identity) {
                        error!("Failed to remove excessive bond: {e:?}");
                    }
                    debug!("Bonds: {:x?}", stack.get_bond_information());
                    conn.raw().disconnect();
                    break;
                }
            }
            _ => log::warn!("unhandled connection event"),
        }
    }
    Ok(())
}

async fn advertise_task<'a, 'b, C: Controller>(
    peripheral: &mut Peripheral<'a, C, DefaultPacketPool>,
    server: &'b Server<'_>,
) -> Result<GattConnection<'a, 'b, DefaultPacketPool>, BleHostError<C::Error>> {
    info!("adv task running");
    let mut adv_data = [0u8; 31];
    let service_uuid: [u8; 16] = DOOR_SERVICE_UUID.to_ne_bytes();
    // FIXME: for some reason, the service uuid is reversed in advertisements
    // The macro reverses the UUID internally...
    // FIXED by to_ne_bytes...
    //service_uuid.reverse();

    AdStructure::encode_slice(
        &[
            AdStructure::Flags(LE_GENERAL_DISCOVERABLE | BR_EDR_NOT_SUPPORTED),
            AdStructure::ServiceUuids128(&[service_uuid]),
            AdStructure::CompleteLocalName(BLE_DEVICE_NAME.as_bytes()),
        ],
        &mut adv_data[..],
    )?;

    let advertiser = peripheral
        .advertise(
            &Default::default(),
            Advertisement::ConnectableScannableUndirected {
                adv_data: &adv_data[..],
                scan_data: &[],
            },
        )
        .await?;

    info!("Advertising...");
    let conn = advertiser.accept().await?.with_attribute_server(server)?;
    info!("Connection established");
    Ok(conn)
}

async fn store_bond_info(flash: &mut BlockingAsync<FlashStorage>, bond_info: BondInformation) {
    let val = BondStoreValue(bond_info);
    let mut data_buffer = [0u8; 64];
    log_error(
        "store bond info",
        store_item(
            flash,
            MAP_FLASH_RANGE,
            &mut NoCache::new(),
            &mut data_buffer,
            &BondStoreKey,
            &val,
        ),
    )
    .await;
}

async fn load_bond_info(flash: &mut BlockingAsync<FlashStorage>) -> Option<BondInformation> {
    let mut data_buffer = [0u8; 64];
    let raw: Option<BondStoreValue> = fetch_item(
        flash,
        MAP_FLASH_RANGE,
        &mut NoCache::new(),
        &mut data_buffer,
        &BondStoreKey,
    )
    .await
    .map_err(|e| {
        error!("Failed to load bond info: {e:?}");
    })
    .ok()
    .flatten();
    raw.map(|v| v.0)
}

#[derive(Debug, PartialEq, Eq, Clone)]
struct BondStoreKey;

const BOND_STORE_VALUE_NAME: &str = "BOND";

impl Key for BondStoreKey {
    fn serialize_into(
        &self,
        mut buffer: &mut [u8],
    ) -> Result<usize, sequential_storage::map::SerializationError> {
        buffer
            .write(BOND_STORE_VALUE_NAME.as_bytes())
            .map_err(|_| SerializationError::InvalidData)
    }
    fn deserialize_from(buffer: &[u8]) -> Result<(Self, usize), SerializationError> {
        if buffer.starts_with(BOND_STORE_VALUE_NAME.as_bytes()) {
            Ok((Self, BOND_STORE_VALUE_NAME.len()))
        } else {
            Err(SerializationError::InvalidData)
        }
    }

    fn get_len(_: &[u8]) -> Result<usize, SerializationError> {
        Ok(BOND_STORE_VALUE_NAME.len())
    }
}

#[derive(Debug, serde::Deserialize, serde::Serialize, PartialEq, Eq)]
struct BondInfoRaw {
    long_term_key: u128,
    bd_addr: [u8; 6],
    identity_resolving_key: u128,
}

impl From<BondInformation> for BondInfoRaw {
    fn from(value: BondInformation) -> Self {
        Self {
            long_term_key: value.ltk.0,
            bd_addr: value.identity.bd_addr.into_inner(),
            identity_resolving_key: value.identity.irk.expect("must ensure irk present").0,
        }
    }
}

impl From<BondInfoRaw> for BondInformation {
    fn from(value: BondInfoRaw) -> Self {
        BondInformation {
            ltk: LongTermKey::new(value.long_term_key),
            identity: Identity {
                bd_addr: BdAddr::new(value.bd_addr),
                irk: Some(IdentityResolvingKey::new(value.identity_resolving_key)),
            },
        }
    }
}

struct BondStoreValue(BondInformation);

impl<'d> Value<'d> for BondStoreValue {
    fn serialize_into(&self, buffer: &mut [u8]) -> Result<usize, SerializationError> {
        let raw: BondInfoRaw = self.0.to_owned().into();
        postcard::to_slice(&raw, buffer)
            .map_err(|_| SerializationError::InvalidFormat)
            .map(|s| s.len())
    }
    fn deserialize_from(buffer: &'d [u8]) -> Result<Self, SerializationError>
    where
        Self: Sized,
    {
        let raw: BondInfoRaw =
            postcard::from_bytes(buffer).map_err(|_| SerializationError::InvalidFormat)?;
        Ok(Self(raw.into()))
    }
}
