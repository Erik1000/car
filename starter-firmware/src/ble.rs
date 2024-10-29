use core::{
    convert::Infallible,
    fmt::{Debug, Display},
    future::Future,
};

use appearance::GENERIC_UNKNOWN;
use embassy_futures::join::join3;
use embassy_time::Timer;
use log::{error, info, warn};
use trouble_host::prelude::*;

use crate::{relay::SIGNAL_ENGINE_STATE, schema::EngineState};

/// Size of L2CAP packets (ATT MTU is this - 4)
const L2CAP_MTU: usize = 251;

/// Max number of connections
const CONNECTIONS_MAX: usize = 1;

/// Max number of L2CAP channels.
const L2CAP_CHANNELS_MAX: usize = 2; // Signal + att

const MAX_ATTRIBUTES: usize = 10;

type Resources<C> = HostResources<C, CONNECTIONS_MAX, L2CAP_CHANNELS_MAX, L2CAP_MTU>;

pub const ADDRESS: [u8; 6] = [0x94, 0xf1, 0xa0, 0x77, 0x4b, 0x6e];

pub const KEY_SERVICE_UUID: [u8; 16] = [
    0x0e, 0x35, 0x35, 0x31, 0x51, 0x59, 0x42, 0xa0, 0x92, 0xff, 0x38, 0xe9, 0xe4, 0x9a, 0xb7, 0xd1,
];

// FIXME: for some reason, if the name is longer, the advertisements fails, e.g. `CarStarter` wont work
pub const BLE_NAME: &str = "Car";

#[gatt_service(uuid = "0e353531-5159-42a0-92ff-38e9e49ab7d1")]
struct EngineService {
    #[characteristic(uuid = "13d24b59-3d13-4ef7-98db-e174869078e0", read, notify, write)]
    engine_state: EngineState,
}

#[gatt_server(attribute_data_size = 10)]
struct Server {
    engine_service: EngineService,
}

pub async fn run<C: Controller>(controller: C) {
    let address = Address::random(ADDRESS);

    let mut resources = Resources::new(PacketQos::None);
    let (stack, peripheral, _, runner) = trouble_host::new(controller, &mut resources)
        .set_random_address(address)
        .build();

    let server = Server::new_with_config(
        stack,
        GapConfig::Peripheral(PeripheralConfig {
            name: BLE_NAME,
            appearance: &GENERIC_UNKNOWN,
        }),
    );

    let _ = join3(
        log_error("ble_task", ble_task(runner)),
        log_error("gatt_task", gatt_task(&server)),
        log_error("advertise_task", advertise_task(peripheral, &server)),
    )
    .await;
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
    runner.run().await.unwrap();
    Ok(())
}

async fn gatt_task<C: Controller>(
    server: &Server<'_, '_, C>,
) -> Result<Infallible, BleHostError<C::Error>> {
    info!("gatt task running");
    loop {
        match server.next().await {
            Ok(GattEvent::Write { handle, connection }) => {
                match server.get(handle, |value| {
                    let engine_state = match value[0] {
                        0 => EngineState::Off,
                        1 => EngineState::Radio,
                        2 => EngineState::Engine,
                        3 => EngineState::Running,
                        _ => {
                            warn!("Got invalid engine state");
                            return value[0];
                        }
                    };
                    SIGNAL_ENGINE_STATE.signal(engine_state);
                    value[0]
                }) {
                    Ok(v) => match server.notify(handle, &connection, &[v]).await {
                        Ok(_) => (),
                        Err(e) => error!("Error notifying: {e:?}"),
                    },
                    Err(e) => warn!("GATT write error: {e:?}"),
                }
            }
            Ok(GattEvent::Read {
                handle,
                connection: _,
            }) => {
                info!("[gatt] Read event on {:?}", handle);
            }
            Err(e) => {
                error!("[gatt] Error processing GATT events: {:?}", e);
            }
        }
    }
}

async fn advertise_task<C: Controller>(
    mut peripheral: Peripheral<'_, C>,
    server: &Server<'_, '_, C>,
) -> Result<(), BleHostError<C::Error>> {
    info!("adv task running");
    let mut adv_data = [0u8; 31];
    let mut scan_data = [0u8; 31];
    let mut service_uuid: [u8; 16] = KEY_SERVICE_UUID;
    // FIXME: for some reason, the service uuid is reversed in advertisements
    service_uuid.reverse();

    AdStructure::encode_slice(
        &[
            AdStructure::Flags(LE_GENERAL_DISCOVERABLE | BR_EDR_NOT_SUPPORTED),
            AdStructure::ServiceUuids128(&[Uuid::from_slice(&service_uuid[..])]),
            //AdStructure::CompleteLocalName(BLE_NAME.as_bytes()),
        ],
        &mut adv_data[..],
    )?;

    AdStructure::encode_slice(
        &[AdStructure::CompleteLocalName(BLE_NAME.as_bytes())],
        &mut scan_data,
    )?;

    loop {
        info!("[adv] advertising");
        info!("Length of packet {}", adv_data.len());
        let mut advertiser = peripheral
            .advertise(
                &Default::default(),
                Advertisement::ConnectableScannableUndirected {
                    adv_data: &adv_data[..],
                    scan_data: &[],
                },
            )
            .await?;
        info!("sending adv: {adv_data:x?}");

        let conn = advertiser.accept().await?;
        while conn.is_connected() {
            Timer::after_secs(1).await;
        }
        info!("[adv] connection established");
    }
}
