use enumset::enum_set;
use std::sync::{Arc, Condvar, Mutex};

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
    hal::{
        delay::FreeRtos,
        gpio::{Gpio14, Gpio25, Gpio26, Gpio27, Gpio32, Gpio33, Output, PinDriver},
    },
    nvs::{EspNvs, NvsDefault},
    sys::{EspError, ESP_FAIL},
};

use log::*;

const APP_ID: u16 = 0;
const MAX_CONNECTIONS: usize = 2;
const BLE_DEVICE_NAME: &str = "EriksDoorController";

// Service UUID for Door Controller
pub const DOOR_SERVICE_UUID: u128 = 0x5eb5b1175231409ea1cab7689f488473;

// Used to initiate OTA update boot
pub const OTA_CHAR_UUID: u128 = 0xe32a319fcfa44838aac359fde6058ee1;

// relay 1 and 2
// 1 GPIO32
// 2 GPIO33
// Recv only
pub const DOOR_CHAR_UUID: u128 = 0x446f5ef8e88940988444e82331c92339;

// Relay 3 and 4
// 3 GPIO25
// 4 GPIO26
// Recv only
pub const WINDOW_LEFT_CHAR_UUID: u128 = 0xb163c9c8b1ac445a8232b7b462bf6b91;

// Relay 5 and 6
// 5 GPIO27
// 6 GPIO14
// Recv only
pub const WINDOW_RIGHT_CHAR_UUID: u128 = 0x8f738eeebbb74cce8b82726a56532bdc;

// Name the types as they are used in the example to get shorter type signatures in the various functions below.
// note that - rather than `Arc`s, you can use regular references as well, but then you have to deal with lifetimes
// and the signatures below will not be `'static`.
type ExBtDriver = BtDriver<'static, Ble>;
type ExEspBleGap = Arc<EspBleGap<'static, Ble, Arc<ExBtDriver>>>;
type ExEspGatts = Arc<EspGatts<'static, Ble, Arc<ExBtDriver>>>;

/// Stores GPIO pins to control the relays
pub struct Controller<'d> {
    door_open: PinDriver<'d, Gpio32, Output>,
    door_close: PinDriver<'d, Gpio33, Output>,
    window_left_up: PinDriver<'d, Gpio25, Output>,
    window_left_down: PinDriver<'d, Gpio26, Output>,
    window_right_up: PinDriver<'d, Gpio27, Output>,
    window_right_down: PinDriver<'d, Gpio14, Output>,
}

pub fn start(nvs: EspNvs<NvsDefault>, bt: Arc<BtDriver<'static, Ble>>) -> anyhow::Result<()> {
    let server = BleServer::new(
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
    recv_handle: Option<Handle>,
    ind_handle: Option<Handle>,
    ind_cccd_handle: Option<Handle>,
    connections: Vec<Connection>,
    response: GattResponse,
    ind_confirmed: Option<BdAddr>,
}

#[derive(Clone)]
pub struct BleServer {
    nvs: Arc<EspNvs<NvsDefault>>,
    gap: ExEspBleGap,
    gatts: ExEspGatts,
    state: Arc<Mutex<State>>,
    condvar: Arc<Condvar>,
}

impl BleServer {
    pub fn new(nvs: EspNvs<NvsDefault>, gap: ExEspBleGap, gatts: ExEspGatts) -> Self {
        Self {
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
        for peer_index in 0..MAX_CONNECTIONS {
            // Propagate this data to all clients which are connected
            // and have subscribed to our indication characteristic

            let mut state = self.state.lock().unwrap();

            loop {
                if state.connections.len() <= peer_index {
                    // We've send to everybody who is connected
                    break;
                }

                let Some(gatt_if) = state.gatt_if else {
                    // We lost the gatt interface in the meantime
                    break;
                };

                let Some(ind_handle) = state.ind_handle else {
                    // We lost the indication handle in the meantime
                    break;
                };

                if state.ind_confirmed.is_none() {
                    let conn = &state.connections[peer_index];

                    self.gatts
                        .indicate(gatt_if, conn.conn_id, ind_handle, data)?;

                    state.ind_confirmed = Some(conn.peer);
                    let conn = &state.connections[peer_index];

                    info!("Indicated data to {}", conn.peer);
                    break;
                } else {
                    state = self.condvar.wait(state).unwrap();
                }
            }
        }

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
                service_handle,
                char_uuid,
            } => {
                self.check_gatt_status(status)?;
                self.register_characteristic(service_handle, attr_handle, char_uuid)?;
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
            service_uuid: Some(BtUuid::uuid128(DOOR_CHAR_UUID)),
            // service_data: todo!(),
            // manufacturer_data: todo!(),
            ..Default::default()
        })?;
        self.gatts.create_service(
            gatt_if,
            &GattServiceId {
                id: GattId {
                    uuid: BtUuid::uuid128(DOOR_CHAR_UUID),
                    inst_id: 0,
                },
                is_primary: true,
            },
            8,
        )?;

        Ok(())
    }

    /// Delete the service
    /// Called from within the event callback once we are notified that the GATTS app is deleted
    fn delete_service(&self, service_handle: Handle) -> Result<(), EspError> {
        let mut state = self.state.lock().unwrap();

        if state.service_handle == Some(service_handle) {
            state.recv_handle = None;
            state.ind_handle = None;
            state.ind_cccd_handle = None;
        }

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
            permissions: enum_set!(Permission::Write | Permission::Read),
            properties: enum_set!(Property::Write | Property::Read),
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
        let indicate_char = {
            let mut state = self.state.lock().unwrap();

            if state.service_handle != Some(service_handle) {
                false
            } else if char_uuid == BtUuid::uuid128(DOOR_CHAR_UUID) {
                state.recv_handle = Some(attr_handle);

                false
            } else if char_uuid == BtUuid::uuid128(DOOR_CHAR_UUID) {
                state.ind_handle = Some(attr_handle);

                true
            } else {
                false
            }
        };

        if indicate_char {
            self.gatts.add_descriptor(
                service_handle,
                &GattDescriptor {
                    uuid: BtUuid::uuid16(0x2902), // CCCD
                    permissions: enum_set!(Permission::Read | Permission::Write),
                },
            )?;
        }

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
        let mut state = self.state.lock().unwrap();

        if descr_uuid == BtUuid::uuid16(0x2902) // CCCD UUID
                && state.service_handle == Some(service_handle)
        {
            state.ind_cccd_handle = Some(attr_handle);
        }

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
        let mut state = self.state.lock().unwrap();

        let recv_handle = state.recv_handle;
        let ind_cccd_handle = state.ind_cccd_handle;

        let Some(conn) = state
            .connections
            .iter_mut()
            .find(|conn| conn.conn_id == conn_id)
        else {
            return Ok(false);
        };

        if Some(handle) == ind_cccd_handle {
            // Subscribe or unsubscribe to our indication characteristic

            if offset == 0 && value.len() == 2 {
                let value = u16::from_le_bytes([value[0], value[1]]);
                if value == 0x02 {
                    if !conn.subscribed {
                        conn.subscribed = true;
                        self.on_subscribed(conn.peer);
                    }
                } else if conn.subscribed {
                    conn.subscribed = false;
                    self.on_unsubscribed(conn.peer);
                }
            }
        } else if Some(handle) == recv_handle {
            // Receive data on the recv characteristic

            self.on_recv(addr, value, offset, conn.mtu);
        } else {
            return Ok(false);
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
        if state.ind_confirmed.is_none() {
            // Should not happen: means we have received a confirmation for
            // an indication we did not send.
            unreachable!();
        }

        state.ind_confirmed = None; // So that the main loop can send the next indication
        self.condvar.notify_all();

        Ok(())
    }

    fn check_esp_status(&self, status: Result<(), EspError>) {
        if let Err(e) = status {
            warn!("Got status: {:?}", e);
        }
    }

    fn check_bt_status(&self, status: BtStatus) -> Result<(), EspError> {
        if !matches!(status, BtStatus::Success) {
            warn!("Got status: {:?}", status);
            Err(EspError::from_infallible::<ESP_FAIL>())
        } else {
            Ok(())
        }
    }

    fn check_gatt_status(&self, status: GattStatus) -> Result<(), EspError> {
        if !matches!(status, GattStatus::Ok) {
            warn!("Got status: {:?}", status);
            Err(EspError::from_infallible::<ESP_FAIL>())
        } else {
            Ok(())
        }
    }
}
