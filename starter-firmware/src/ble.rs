use core::{
    convert::Infallible,
    fmt::{Debug, Display},
    future::Future,
};

use alloc::borrow::ToOwned;
use embassy_embedded_hal::adapter::BlockingAsync;
use embassy_futures::{
    join::join,
    select::{select, Either},
};
use embedded_io::Write;
use esp_hal::rng::Trng;
use esp_storage::FlashStorage;
use log::{debug, error, info, warn};
use postcard::{from_bytes, to_slice};
use sequential_storage::{
    cache::NoCache,
    map::{fetch_item, store_item, Key, SerializationError, Value},
};
use trouble_host::{prelude::*, BondInformation, IdentityResolvingKey, LongTermKey};

use crate::{
    relay::{SIGNAL_BLE_STATE_CHANGE, SIGNAL_ENGINE_STATE},
    schema::EngineState,
    MAP_FLASH_RANGE,
};

/// Max number of connections
const CONNECTIONS_MAX: usize = 1;

/// Max number of L2CAP channels.
const L2CAP_CHANNELS_MAX: usize = 2; // Signal + att

//const MAX_ATTRIBUTES: usize = 10;

pub const ADDRESS: [u8; 6] = [0x94, 0xf1, 0xa0, 0x77, 0x4b, 0x6e];

pub const ENGINE_SERVICE_UUID: [u8; 16] = [
    0x0e, 0x35, 0x35, 0x31, 0x51, 0x59, 0x42, 0xa0, 0x92, 0xff, 0x38, 0xe9, 0xe4, 0x9a, 0xb7, 0xd1,
];

// FIXME: for some reason, if the name is longer, the advertisements fails, e.g. `CarStarter` wont work
pub const BLE_NAME: &str = "Car";

#[gatt_service(uuid = "0e353531-5159-42a0-92ff-38e9e49ab7d1")]
struct EngineService {
    #[characteristic(uuid = "13d24b59-3d13-4ef7-98db-e174869078e0", read, notify, write)]
    engine_state: EngineState,
}

#[gatt_server]
struct Server {
    engine_service: EngineService,
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

    if let Some(bond_info) = load_bond_info(&mut flash).await {
        stack.add_bond_information(bond_info)?;
    }

    let server = Server::new_with_config(GapConfig::Peripheral(PeripheralConfig {
        name: BLE_NAME,
        appearance: &trouble_host::prelude::appearance::control_device::GENERIC_CONTROL_DEVICE,
    }))
    .map_err(|_| Error::Other)?;
    log::info!("Bonded devices: {:#?}", stack.get_bond_information());
    let _ = join(log_error("ble_task", ble_task(runner)), async {
        loop {
            log::info!("Repeat");
            match advertise_task(&mut peripheral, &server).await {
                Ok(conn) => {
                    let a = gatt_task(&server, &conn, &stack, &mut flash);
                    let b = notify_task(&server, &conn);
                    match select(a, b).await {
                        Either::First(f) => {
                            if let Err(e) = f {
                                return Err(e);
                            } else {
                                continue;
                            }
                        }
                        Either::Second(s) => {
                            return s;
                        }
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

async fn gatt_task<'b>(
    server: &'b Server<'_>,
    conn: &GattConnection<'_, 'b, DefaultPacketPool>,
    stack: &Stack<'_, impl Controller, DefaultPacketPool>,
    flash: &mut BlockingAsync<FlashStorage>,
) -> Result<(), Error> {
    info!("gatt task running");
    let engine_state = &server.engine_service.engine_state;
    loop {
        match conn.next().await {
            GattConnectionEvent::Gatt { event } => match event? {
                GattEvent::Read(event) => {
                    if event.handle() == engine_state.handle {
                        let value = server.get(engine_state)?;
                        log::info!("Read value {value:?}");
                    }
                    if conn.raw().encrypted() {
                        event.accept()?.send().await;
                    } else {
                        info!("Read rejected due to insufficient encryption");
                        event
                            .reject(AttErrorCode::INSUFFICIENT_ENCRYPTION)?
                            .send()
                            .await;
                    }
                }
                GattEvent::Write(event) => {
                    if conn.raw().encrypted() {
                        if event.handle() == engine_state.handle {
                            let val = match EngineState::from_gatt(event.data()) {
                                Ok(val) => val,
                                Err(_) => {
                                    log::error!("Rejected write event: {:?}", event.data());
                                    event.reject(AttErrorCode::VALUE_NOT_ALLOWED)?;
                                    continue;
                                }
                            };
                            SIGNAL_BLE_STATE_CHANGE.signal(val);
                            event.accept()?.send().await;
                        }
                    } else {
                        warn!("Write rejected due to unencrypted connection");
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
    log::info!("gatt task finished");
    Ok(())
}

async fn advertise_task<'a, 'b, C: Controller>(
    peripheral: &mut Peripheral<'a, C, DefaultPacketPool>,
    server: &'b Server<'_>,
) -> Result<GattConnection<'a, 'b, DefaultPacketPool>, BleHostError<C::Error>> {
    info!("adv task running");
    let mut adv_data = [0u8; 31];
    let mut service_uuid: [u8; 16] = ENGINE_SERVICE_UUID;
    // FIXME: for some reason, the service uuid is reversed in advertisements
    // The macro reverses the UUID internally...
    service_uuid.reverse();

    AdStructure::encode_slice(
        &[
            AdStructure::Flags(LE_GENERAL_DISCOVERABLE | BR_EDR_NOT_SUPPORTED),
            AdStructure::ServiceUuids128(&[service_uuid]),
            AdStructure::CompleteLocalName(BLE_NAME.as_bytes()),
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
    info!("Got connection from {:x?}", conn.raw().peer_address());
    Ok(conn)
}

async fn notify_task(
    server: &Server<'_>,
    conn: &GattConnection<'_, '_, DefaultPacketPool>,
) -> Result<Infallible, Error> {
    let engine_state_attr = &server.engine_service.engine_state;
    loop {
        let new_state = SIGNAL_ENGINE_STATE.wait().await;
        // notify connected clients and update engine state in BLE value store
        if conn.raw().encrypted() {
            engine_state_attr.notify(conn, &new_state).await?;
        } else {
            warn!("Not notifying because connection is not encrypted")
        }
    }
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
        to_slice(&raw, buffer)
            .map_err(|_| SerializationError::InvalidFormat)
            .map(|s| s.len())
    }
    fn deserialize_from(buffer: &'d [u8]) -> Result<Self, SerializationError>
    where
        Self: Sized,
    {
        let raw: BondInfoRaw = from_bytes(buffer).map_err(|_| SerializationError::InvalidFormat)?;
        Ok(Self(raw.into()))
    }
}
