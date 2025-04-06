use std::{task::Poll, thread::sleep, time::Duration};

use anyhow::anyhow;
use esp_idf_svc::{
    eventloop::EspSystemEventLoop,
    hal::{modem::Modem, task::block_on},
    http::{
        server::{fn_handler, Connection, EspHttpConnection, EspHttpServer, Handler, Request},
        Method,
    },
    io::Write,
    nvs::EspDefaultNvsPartition,
    ota::{EspFirmwareInfoLoad, EspOta, FirmwareInfo},
    wifi::{AccessPointConfiguration, BlockingWifi, Configuration, EspWifi},
};
use futures_core::Stream;
use multer::{bytes::Bytes, Multipart};

pub const OWN_SSID: &str = "Door-Controller-OTA";
pub const OWN_PASSWORD: &str = "followthewhiterabbit";

//const MAX_LEN: usize = 128;
const STACK_SIZE: usize = 10240;

const CHANNEL: u8 = 11;

pub fn run_ota_update_mode(
    modem: Modem,
    sys_loop: EspSystemEventLoop,
    nvs: EspDefaultNvsPartition,
) -> anyhow::Result<()> {
    let mut wifi = BlockingWifi::wrap(EspWifi::new(modem, sys_loop.clone(), Some(nvs))?, sys_loop)?;
    let wifi_conf = Configuration::AccessPoint(AccessPointConfiguration {
        channel: CHANNEL,
        password: OWN_PASSWORD.parse().unwrap(),
        ssid: OWN_SSID.parse().unwrap(),
        auth_method: esp_idf_svc::wifi::AuthMethod::WPA2WPA3Personal,
        ..Default::default()
    });

    wifi.set_configuration(&wifi_conf)?;
    wifi.start()?;

    let server = run_webserver()?;
    log::info!("Waiting for webserver");
    core::mem::forget(server);
    sleep(Duration::from_secs(60 * 5));
    esp_idf_svc::hal::reset::restart();
}

pub fn run_webserver() -> anyhow::Result<EspHttpServer<'static>> {
    let server_conf = esp_idf_svc::http::server::Configuration {
        stack_size: STACK_SIZE,
        ..Default::default()
    };

    let mut server = EspHttpServer::new(&server_conf)?;
    server.handler("/", Method::Get, fn_handler(handle_get))?;
    server.handler("/upload", Method::Post, PostHandler)?;
    Ok(server)
}

fn handle_get(req: Request<&mut EspHttpConnection<'_>>) -> anyhow::Result<()> {
    let mut resp = req.into_ok_response()?;
    resp.write_all(include_bytes!("updater/index.html"))?;
    Ok(())
}

struct PostHandler;

impl<C> Handler<C> for PostHandler
where
    C: Connection,
{
    type Error = anyhow::Error;

    fn handle<'a>(&self, conn: &mut C) -> anyhow::Result<()> {
        let mut ota = EspOta::new()?;
        let mut update = ota.initiate_update()?;

        let update_info_load = EspFirmwareInfoLoad {};
        let mut update_info = FirmwareInfo {
            version: Default::default(),
            released: Default::default(),
            description: Default::default(),
            signature: Default::default(),
            download_id: Default::default(),
        };

        let ct = conn
            .header("Content-Type")
            .ok_or(anyhow!("no content type header"))?
            .to_owned();
        let boundary = ct.split_once("boundary=").ok_or(anyhow!("no boundary"))?.1;
        let stream = StreamChunker {
            last_pending: 0,
            inner: conn,
            buffer: [0; 1024],
            total_read: 0,
        };

        let mut multipart = Multipart::new(stream, boundary);
        let mut field = block_on(multipart.next_field())?.ok_or(anyhow!("no file uploaded"))?;
        let mut update_data_found = false;
        log::info!("File name {:?}", field.file_name());
        loop {
            match block_on(field.chunk()) {
                Ok(Some(bytes)) => {
                    update.write(&bytes)?;
                    if !update_data_found {
                        let finished = match update_info_load.fetch(&bytes, &mut update_info) {
                            Ok(t) => t,
                            Err(e) => {
                                log::error!("OTA Error: {e}");
                                false
                            }
                        };
                        update_data_found = finished;
                        log::info!("Update state: {finished:?}")
                    }
                    drop(bytes);
                }
                Ok(None) => {
                    log::info!("finished reading");
                    drop(field);
                    drop(multipart);
                    conn.initiate_response(200, Some("updated"), &[])
                        .map_err(|e| anyhow!("Error: {e:#?}"))?;
                    break;
                }
                Err(e) => Err(e)?,
            }
        }
        update.complete()?;
        log::warn!("Update completed: {update_info:#?}");
        esp_idf_svc::hal::reset::restart();
    }
}

// SAFETY: I think this is safe because it will not be sent to another thread
// This implementation is only needed to satisfy some stupid trait bound in the
// multipart library
unsafe impl<C> Send for StreamChunker<'_, C> where C: Connection {}
pub struct StreamChunker<'a, C> {
    inner: &'a mut C,
    buffer: [u8; 1024],
    last_pending: u8,
    total_read: usize,
}

impl<C> Stream for StreamChunker<'_, C>
where
    C: Connection,
{
    type Item = anyhow::Result<Bytes>;
    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        let me = self.get_mut();

        // The multipart library reads chunks until Pending is returned
        // If the stream is always read (which it is in this implementation),
        // it will load everything at once which will lead to out of memory errors
        // this way at most 4 * 1024 should be read which works fine
        // https://github.com/rwf2/multer/issues/62#issuecomment-2082388404
        me.last_pending += 1;
        if me.last_pending > 3 {
            me.last_pending = 0;
            return Poll::Pending;
        }

        let read = match me.inner.read(&mut me.buffer) {
            Ok(read) => read,
            Err(e) => return Poll::Ready(Some(Err(anyhow!("Error: {e:#?}")))),
        };
        me.total_read += read;
        log::info!("read {read} bytes from connection");
        if read == 0 {
            Poll::Ready(None)
        } else {
            let bytes: Bytes = me.buffer[0..read].iter().cloned().collect();
            Poll::Ready(Some(Ok(bytes)))
        }
    }
}

impl<C> Drop for StreamChunker<'_, C> {
    fn drop(&mut self) {
        log::info!("Total read in stream: {}", self.total_read);
    }
}
