use core::{
    convert::Infallible,
    fmt::{Debug, Display},
    future::Future,
};

use embassy_futures::{
    join::join,
    select::{select, Either},
};
use esp_hal::rng::Trng;
use log::{error, info};
use trouble_host::prelude::*;

use crate::{
    relay::{SIGNAL_BLE_STATE_CHANGE, SIGNAL_ENGINE_STATE},
    schema::EngineState,
};

/// Size of L2CAP packets (ATT MTU is this - 4)
const L2CAP_MTU: usize = 251;

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

pub async fn run<C: Controller>(controller: C, mut rng: Trng<'_>) -> Result<(), Error> {
    let address = Address::random(ADDRESS);

    let mut resources: HostResources<CONNECTIONS_MAX, L2CAP_CHANNELS_MAX, L2CAP_MTU> =
        HostResources::new();
    let stack = trouble_host::new(controller, &mut resources)
        .set_random_address(address)
        .set_random_generator_seed(&mut rng);

    let Host {
        mut peripheral,
        runner,
        ..
    } = stack.build();

    let server = Server::new_with_config(GapConfig::Peripheral(PeripheralConfig {
        name: BLE_NAME,
        appearance: &trouble_host::prelude::appearance::control_device::GENERIC_CONTROL_DEVICE,
    }))
    .map_err(|_| Error::Other)?;

    let _ = join(log_error("ble_task", ble_task(runner)), async {
        loop {
            log::info!("Repeat");
            match advertise_task(&mut peripheral, &server).await {
                Ok(conn) => {
                    let a = gatt_task(&server, &conn);
                    let b = notify_task(&server, &conn);
                    match select(a, b).await {
                        Either::First(f) => {
                            // can only ever return error because Ok is infallible
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
async fn ble_task<C: Controller>(mut runner: Runner<'_, C>) -> Result<(), BleHostError<C::Error>> {
    runner.run().await?;
    Ok(())
}

async fn gatt_task(server: &Server<'_>, conn: &GattConnection<'_, '_>) -> Result<(), Error> {
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
                    // if conn.raw().encrypted() {
                    //     event.accept()?.send().await;
                    // } else {
                    //     event
                    //         .reject(AttErrorCode::INSUFFICIENT_ENCRYPTION)?
                    //         .send()
                    //         .await;
                    // }
                    event.accept()?.send().await;
                }
                GattEvent::Write(event) => {
                    if event.handle() == engine_state.handle {
                        //if conn.raw().encrypted() {
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
                        // } else {
                        //     event
                        //         .reject(AttErrorCode::INSUFFICIENT_ENCRYPTION)?
                        //         .send()
                        //         .await;
                        // }
                    }
                }
            },
            GattConnectionEvent::Disconnected { reason } => {
                log::warn!("Disconnected: {reason:?}");
                break;
            }
            _ => log::warn!("unhandled connection event"),
        }
    }
    log::info!("gatt task finished");
    Ok(())
}

async fn advertise_task<'a, 'b, C: Controller>(
    peripheral: &mut Peripheral<'a, C>,
    server: &'b Server<'_>,
) -> Result<GattConnection<'a, 'b>, BleHostError<C::Error>> {
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
    info!("Connection established");
    Ok(conn)
}

async fn notify_task(
    server: &Server<'_>,
    conn: &GattConnection<'_, '_>,
) -> Result<Infallible, Error> {
    let engine_state_attr = &server.engine_service.engine_state;
    loop {
        let new_state = SIGNAL_ENGINE_STATE.wait().await;
        // notify connected clients and update engine state in BLE value store
        engine_state_attr.notify(conn, &new_state).await?;
    }
}
