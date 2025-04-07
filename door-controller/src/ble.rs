use enumset::enum_set;
use std::sync::{mpsc::Sender, Arc, Condvar, Mutex};

use esp_idf_svc::{
    bt::{
        ble::{
            gap::{AdvConfiguration, BleGapEvent, EspBleGap},
            gatt::{
                server::{ConnectionId, EspGatts, GattsEvent, TransferId},
                AutoResponse, GattCharacteristic, GattDescriptor, GattId, GattInterface,
                GattResponse, GattServiceId, GattStatus, Handle, Permission, Property,
            },
        },
        BdAddr, Ble, BtDriver, BtStatus, BtUuid,
    },
    hal::delay::FreeRtos,
    nvs::{EspNvs, NvsDefault},
    ota::EspOta,
    sys::{EspError, ESP_FAIL},
};

use log::*;

use crate::controller::Operation;

const APP_ID: u16 = 0;
const MAX_CONNECTIONS: usize = 2;

// if too long it will leak into the advertisement packets
const BLE_DEVICE_NAME: &str = "DCtrl";

// Service UUID for Door Controller
pub const DOOR_SERVICE_UUID: u128 = 0x5eb5b1175231409ea1cab7689f488473;

// Used to initiate OTA update boot
pub const OTA_CHAR_UUID: u128 = 0xe32a319fcfa44838aac359fde6058ee1;

// relay 1 and 2 and 3
// 1 GPIO32
// 2 GPIO33
// 3 GPIO25
// Recv only
pub const DOOR_CHAR_UUID: u128 = 0x446f5ef8e88940988444e82331c92339;

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

// Name the types as they are used in the example to get shorter type signatures in the various functions below.
// note that - rather than `Arc`s, you can use regular references as well, but then you have to deal with lifetimes
// and the signatures below will not be `'static`.
type ExBtDriver = BtDriver<'static, Ble>;
type ExEspBleGap = Arc<EspBleGap<'static, Ble, Arc<ExBtDriver>>>;
type ExEspGatts = Arc<EspGatts<'static, Ble, Arc<ExBtDriver>>>;
pub fn start(
    tx: Sender<Operation>,
    nvs: EspNvs<NvsDefault>,
    bt: Arc<BtDriver<'static, Ble>>,
) -> anyhow::Result<()> {
    let server = BleServer::new(
        tx,
        nvs,
        Arc::new(EspBleGap::new(bt.clone())?),
        Arc::new(EspGatts::new(bt.clone())?),
    );
    info!("BLE Gap and Gatts initialized");

    let gap_server = server.clone();

    server.gap.subscribe(move |event| {
        gap_server.check_esp_status(gap_server.on_gap_event(event));
    })?;

    let gatts_server = server.clone();

    server.gatts.subscribe(move |(gatt_if, event)| {
        gatts_server.check_esp_status(gatts_server.on_gatts_event(gatt_if, event))
    })?;

    info!("BLE Gap and Gatts subscriptions initialized");

    server.gatts.register_app(APP_ID)?;

    info!("Gatts BTP app registered");

    loop {
        FreeRtos::delay_ms(10000);
    }
}
#[derive(Debug, Clone)]
struct Connection {
    peer: BdAddr,
    conn_id: Handle,
    subscribed: bool,
    mtu: Option<u16>,
}

#[derive(Default)]
struct State {
    gatt_if: Option<GattInterface>,
    service_handle: Option<Handle>,
    ota_handle: Option<Handle>,
    door_handle: Option<Handle>,
    window_left_handle: Option<Handle>,
    window_right_handle: Option<Handle>,
    connections: Vec<Connection>,
    response: GattResponse,
}

#[derive(Clone)]
pub struct BleServer {
    tx: Sender<Operation>,
    nvs: Arc<EspNvs<NvsDefault>>,
    gap: ExEspBleGap,
    gatts: ExEspGatts,
    state: Arc<Mutex<State>>,
    condvar: Arc<Condvar>,
}

impl BleServer {
    pub fn new(
        tx: Sender<Operation>,
        nvs: EspNvs<NvsDefault>,
        gap: ExEspBleGap,
        gatts: ExEspGatts,
    ) -> Self {
        Self {
            tx,
            nvs: Arc::new(nvs),
            gap,
            gatts,
            state: Arc::new(Mutex::new(Default::default())),
            condvar: Arc::new(Condvar::new()),
        }
    }
}

impl BleServer {
    /// Send (indicate) data to all peers that are currently
    /// subscribed to our indication characteristic
    ///
    /// Uses a Mutex + Condvar to wait until indication confirmation
    /// is received.
    ///
    /// This complexity is necessary only when using indications.
    /// Notifications do not really send confirmation and thus do not
    /// need this synchronization.
    fn indicate(&self, data: &[u8]) -> Result<(), EspError> {
        // for peer_index in 0..MAX_CONNECTIONS {
        //     // Propagate this data to all clients which are connected
        //     // and have subscribed to our indication characteristic

        //     let mut state = self.state.lock().unwrap();

        //     loop {
        //         if state.connections.len() <= peer_index {
        //             // We've send to everybody who is connected
        //             break;
        //         }

        //         let Some(gatt_if) = state.gatt_if else {
        //             // We lost the gatt interface in the meantime
        //             break;
        //         };

        //         let Some(ind_handle) = state.ind_handle else {
        //             // We lost the indication handle in the meantime
        //             break;
        //         };

        //         if state.ind_confirmed.is_none() {
        //             let conn = &state.connections[peer_index];

        //             self.gatts
        //                 .indicate(gatt_if, conn.conn_id, ind_handle, data)?;

        //             state.ind_confirmed = Some(conn.peer);
        //             let conn = &state.connections[peer_index];

        //             info!("Indicated data to {}", conn.peer);
        //             break;
        //         } else {
        //             state = self.condvar.wait(state).unwrap();
        //         }
        //     }
        // }

        Ok(())
    }

    /// Sample callback where the user code can handle a newly-subscribed client
    ///
    /// If the user code just broadcasts the _same_ indication to all subscribed
    /// clients, this callback might not be necessary.
    fn on_subscribed(&self, addr: BdAddr) {
        // Put your custom code here or leave empty
        // `indicate()` will anyway send to all connected clients
        warn!("Client {addr} subscribed - put your custom logic here");
    }

    /// Sample callback where the user code can handle an unsubscribed client
    ///
    /// If the user code just broadcasts the _same_ indication to all subscribed
    /// clients, this callback might not be necessary.
    fn on_unsubscribed(&self, addr: BdAddr) {
        // Put your custom code here
        // `indicate()` will anyway send to all connected clients
        warn!("Client {addr} unsubscribed - put your custom logic here");
    }

    /// Sample callback where the user code can handle received data
    /// For demo purposes, the data is just logged.
    fn on_recv(&self, addr: BdAddr, data: &[u8], offset: u16, mtu: Option<u16>) {
        // Put your custom code here
        warn!("Received data from {addr}: {data:?}, offset: {offset}, mtu: {mtu:?} - put your custom logic here");
    }

    /// The main event handler for the GAP events
    fn on_gap_event(&self, event: BleGapEvent) -> Result<(), EspError> {
        info!("Got event: {event:?}");

        if let BleGapEvent::AdvertisingConfigured(status) = event {
            self.check_bt_status(status)?;
            self.gap.start_advertising()?;
        }

        Ok(())
    }

    /// The main event handler for the GATTS events
    fn on_gatts_event(&self, gatt_if: GattInterface, event: GattsEvent) -> Result<(), EspError> {
        info!("Got event: {event:?}");

        match event {
            GattsEvent::ServiceRegistered { status, app_id } => {
                self.check_gatt_status(status)?;
                if APP_ID == app_id {
                    self.create_service(gatt_if)?;
                }
            }
            GattsEvent::ServiceCreated {
                status,
                service_handle,
                ..
            } => {
                self.check_gatt_status(status)?;
                self.configure_and_start_service(service_handle)?;
            }
            GattsEvent::CharacteristicAdded {
                status,
                attr_handle,
                service_handle: _,
                char_uuid,
            } => {
                let mut state = self.state.lock().unwrap();
                if char_uuid.as_bytes() == OTA_CHAR_UUID.to_le_bytes() {
                    state.ota_handle = Some(attr_handle);
                } else if char_uuid.as_bytes() == DOOR_CHAR_UUID.to_le_bytes() {
                    state.door_handle = Some(attr_handle);
                } else if char_uuid.as_bytes() == WINDOW_LEFT_CHAR_UUID.to_le_bytes() {
                    state.window_left_handle = Some(attr_handle);
                } else if char_uuid.as_bytes() == WINDOW_RIGHT_CHAR_UUID.to_le_bytes() {
                    state.window_right_handle = Some(attr_handle);
                }
                self.check_gatt_status(status)?;
            }
            GattsEvent::DescriptorAdded {
                status,
                attr_handle,
                service_handle,
                descr_uuid,
            } => {
                self.check_gatt_status(status)?;
                self.register_cccd_descriptor(service_handle, attr_handle, descr_uuid)?;
            }
            GattsEvent::ServiceDeleted {
                status,
                service_handle,
            } => {
                self.check_gatt_status(status)?;
                self.delete_service(service_handle)?;
            }
            GattsEvent::ServiceUnregistered {
                status,
                service_handle,
                ..
            } => {
                self.check_gatt_status(status)?;
                self.unregister_service(service_handle)?;
            }
            GattsEvent::Mtu { conn_id, mtu } => {
                self.register_conn_mtu(conn_id, mtu)?;
            }
            GattsEvent::PeerConnected { conn_id, addr, .. } => {
                self.create_conn(conn_id, addr)?;
            }
            GattsEvent::PeerDisconnected { addr, .. } => {
                self.delete_conn(addr)?;
            }
            GattsEvent::Write {
                conn_id,
                trans_id,
                addr,
                handle,
                offset,
                need_rsp,
                is_prep,
                value,
            } => {
                let handled = self.recv(
                    gatt_if, conn_id, trans_id, addr, handle, offset, need_rsp, is_prep, value,
                )?;

                if handled {
                    self.send_write_response(
                        gatt_if, conn_id, trans_id, handle, offset, need_rsp, is_prep, value,
                    )?;
                }
            }
            GattsEvent::Confirm { status, .. } => {
                self.check_gatt_status(status)?;
                self.confirm_indication()?;
            }
            _ => (),
        }

        Ok(())
    }

    /// Create the service and start advertising
    /// Called from within the event callback once we are notified that the GATTS app is registered
    fn create_service(&self, gatt_if: GattInterface) -> Result<(), EspError> {
        self.state.lock().unwrap().gatt_if = Some(gatt_if);

        self.gap.set_device_name(BLE_DEVICE_NAME)?;
        self.gap.set_adv_conf(&AdvConfiguration {
            include_name: true,
            include_txpower: true,
            flag: 2,
            service_uuid: Some(BtUuid::uuid128(DOOR_SERVICE_UUID)),
            // service_data: todo!(),
            // manufacturer_data: todo!(),
            ..Default::default()
        })?;
        self.gatts.create_service(
            gatt_if,
            &GattServiceId {
                id: GattId {
                    uuid: BtUuid::uuid128(DOOR_SERVICE_UUID),
                    inst_id: 0,
                },
                is_primary: true,
            },
            // FIXME: depending on how many attributes we have, this value must be changed but I havent figured out what number we need exactly
            15,
        )?;

        Ok(())
    }

    /// Delete the service
    /// Called from within the event callback once we are notified that the GATTS app is deleted
    fn delete_service(&self, service_handle: Handle) -> Result<(), EspError> {
        Ok(())
    }

    /// Unregister the service
    /// Called from within the event callback once we are notified that the GATTS app is unregistered
    fn unregister_service(&self, service_handle: Handle) -> Result<(), EspError> {
        let mut state = self.state.lock().unwrap();

        if state.service_handle == Some(service_handle) {
            state.gatt_if = None;
            state.service_handle = None;
        }

        Ok(())
    }

    /// Configure and start the service
    /// Called from within the event callback once we are notified that the service is created
    fn configure_and_start_service(&self, service_handle: Handle) -> Result<(), EspError> {
        self.state.lock().unwrap().service_handle = Some(service_handle);

        self.gatts.start_service(service_handle)?;
        self.add_characteristics(service_handle)?;

        Ok(())
    }

    /// Add our two characteristics to the service
    /// Called from within the event callback once we are notified that the service is created
    fn add_characteristics(&self, service_handle: Handle) -> Result<(), EspError> {
        let door_char = GattCharacteristic {
            uuid: BtUuid::uuid128(DOOR_CHAR_UUID),
            permissions: enum_set!(Permission::Write),
            properties: enum_set!(Property::Write),
            max_len: 1,
            auto_rsp: AutoResponse::ByApp,
        };
        let mut left_window_char = door_char.clone();
        left_window_char.uuid = BtUuid::uuid128(WINDOW_LEFT_CHAR_UUID);

        let mut right_window_char = door_char.clone();
        right_window_char.uuid = BtUuid::uuid128(WINDOW_RIGHT_CHAR_UUID);

        self.gatts
            .add_characteristic(service_handle, &door_char, &[])?;
        self.gatts
            .add_characteristic(service_handle, &left_window_char, &[])?;
        self.gatts
            .add_characteristic(service_handle, &right_window_char, &[])?;

        let ota_update_char = GattCharacteristic {
            uuid: BtUuid::uuid128(OTA_CHAR_UUID),
            permissions: enum_set!(Permission::Write),
            properties: enum_set!(Property::Write),
            max_len: 1,
            auto_rsp: AutoResponse::ByApp,
        };
        self.gatts
            .add_characteristic(service_handle, &ota_update_char, &[])?;
        Ok(())
    }

    /// Add the CCCD descriptor
    /// Called from within the event callback once we are notified that a char descriptor is added,
    /// however the method will do something only if the added char is the "indicate" characteristics of course
    fn register_characteristic(
        &self,
        service_handle: Handle,
        attr_handle: Handle,
        char_uuid: BtUuid,
    ) -> Result<(), EspError> {
        Ok(())
    }

    /// Register the CCCD descriptor
    /// Called from within the event callback once we are notified that a descriptor is added,
    fn register_cccd_descriptor(
        &self,
        service_handle: Handle,
        attr_handle: Handle,
        descr_uuid: BtUuid,
    ) -> Result<(), EspError> {
        Ok(())
    }

    /// Receive data from a client
    /// Called from within the event callback once we are notified for the connection MTU
    fn register_conn_mtu(&self, conn_id: ConnectionId, mtu: u16) -> Result<(), EspError> {
        let mut state = self.state.lock().unwrap();

        if let Some(conn) = state
            .connections
            .iter_mut()
            .find(|conn| conn.conn_id == conn_id)
        {
            conn.mtu = Some(mtu);
        }

        Ok(())
    }

    /// Create a new connection
    /// Called from within the event callback once we are notified for a new connection
    fn create_conn(&self, conn_id: ConnectionId, addr: BdAddr) -> Result<(), EspError> {
        log::info!("new connection");
        let added = {
            let mut state = self.state.lock().unwrap();

            if state.connections.len() < MAX_CONNECTIONS {
                state.connections.push(Connection {
                    peer: addr,
                    conn_id,
                    subscribed: false,
                    mtu: None,
                });

                true
            } else {
                false
            }
        };

        if added {
            self.gap.set_conn_params_conf(addr, 10, 20, 0, 400)?;
        }

        Ok(())
    }

    /// Delete a connection
    /// Called from within the event callback once we are notified for a disconnected peer
    fn delete_conn(&self, addr: BdAddr) -> Result<(), EspError> {
        log::info!("disconnected");
        let mut state = self.state.lock().unwrap();

        if let Some(index) = state
            .connections
            .iter()
            .position(|Connection { peer, .. }| *peer == addr)
        {
            state.connections.swap_remove(index);
        }

        Ok(())
    }

    /// A helper method to process a client sending us data to the "recv" characteristic
    #[allow(clippy::too_many_arguments)]
    fn recv(
        &self,
        _gatt_if: GattInterface,
        conn_id: ConnectionId,
        _trans_id: TransferId,
        addr: BdAddr,
        handle: Handle,
        offset: u16,
        _need_rsp: bool,
        _is_prep: bool,
        value: &[u8],
    ) -> Result<bool, EspError> {
        let state = self.state.lock().unwrap();

        if Some(handle) == state.ota_handle {
            info!("Write on OTA: {value:?}");
            match value.first() {
                Some(1) => {
                    self.nvs.set_u8("enter_update", 1)?;
                    esp_idf_svc::hal::reset::restart();
                }
                Some(2) => {
                    self.nvs.set_u8("enter_update", 0)?;
                    EspOta::new()?.mark_running_slot_valid()?;
                }
                _ => (),
            };
        } else if Some(handle) == state.door_handle {
            info!("Write on Door: {value:?}");
            match value.first() {
                Some(0) => {
                    if let Err(e) = self.tx.send(Operation::DoorClose) {
                        error!("Send error: {e}");
                    }
                }
                Some(1) => {
                    if let Err(e) = self.tx.send(Operation::DoorOpen) {
                        error!("Send error: {e}");
                    }
                }
                _ => warn!("Invalid door state sent"),
            }
        } else if Some(handle) == state.window_left_handle {
            info!("Write on window left");
            match value.first() {
                Some(0) => {
                    if let Err(e) = self.tx.send(Operation::WindowLeftUp) {
                        error!("Send error: {e}")
                    }
                }
                Some(1) => {
                    if let Err(e) = self.tx.send(Operation::WindowLeftDown) {
                        error!("Send error: {e}")
                    }
                }
                _ => warn!("Invalid window left state sent"),
            }
        } else if Some(handle) == state.window_right_handle {
            info!("Write on window right");
            match value.first() {
                Some(0) => {
                    if let Err(e) = self.tx.send(Operation::WindowRightUp) {
                        error!("Send error: {e}")
                    }
                }
                Some(1) => {
                    if let Err(e) = self.tx.send(Operation::WindowRightDown) {
                        error!("Send error: {e}")
                    }
                }
                _ => warn!("Invalid window right state sent"),
            }
        } else {
            warn!("Received write on unknown handle {handle:?}");
        }
        Ok(true)
    }

    /// A helper method that sends a response to the peer that just sent us some data on the "recv"
    /// characteristic.
    ///
    /// This is only necessary, because we support write confirmation
    /// (which is the more complex case as compared to unconfirmed writes).
    #[allow(clippy::too_many_arguments)]
    fn send_write_response(
        &self,
        gatt_if: GattInterface,
        conn_id: ConnectionId,
        trans_id: TransferId,
        handle: Handle,
        offset: u16,
        need_rsp: bool,
        is_prep: bool,
        value: &[u8],
    ) -> Result<(), EspError> {
        if !need_rsp {
            return Ok(());
        }

        if is_prep {
            let mut state = self.state.lock().unwrap();

            state
                .response
                .attr_handle(handle)
                .auth_req(0)
                .offset(offset)
                .value(value)
                .map_err(|_| EspError::from_infallible::<ESP_FAIL>())?;

            self.gatts.send_response(
                gatt_if,
                conn_id,
                trans_id,
                GattStatus::Ok,
                Some(&state.response),
            )?;
        } else {
            self.gatts
                .send_response(gatt_if, conn_id, trans_id, GattStatus::Ok, None)?;
        }

        Ok(())
    }

    /// A helper method to handle an indication conrimation.
    /// Basically, we need to notify the "indicate" method that sending the indication was
    /// confirmed, so that it is free to send the next indication.
    fn confirm_indication(&self) -> Result<(), EspError> {
        let mut state = self.state.lock().unwrap();
        self.condvar.notify_all();

        Ok(())
    }

    fn check_esp_status(&self, status: Result<(), EspError>) {
        if let Err(e) = status {
            warn!("Got esp status: {:?}", e);
        }
    }

    fn check_bt_status(&self, status: BtStatus) -> Result<(), EspError> {
        if !matches!(status, BtStatus::Success) {
            warn!("Got bt status: {:?}", status);
            Err(EspError::from_infallible::<ESP_FAIL>())
        } else {
            Ok(())
        }
    }

    fn check_gatt_status(&self, status: GattStatus) -> Result<(), EspError> {
        if !matches!(status, GattStatus::Ok) {
            warn!("Got gatt status: {:?}", status);
            Err(EspError::from_infallible::<ESP_FAIL>())
        } else {
            Ok(())
        }
    }
}
