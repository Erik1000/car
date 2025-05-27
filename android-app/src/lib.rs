use std::convert::Infallible;

use jni::JNIEnv;
use jni_utils as _;

#[macro_use]
extern crate log;

mod ble;
mod schema;
mod sms;

#[allow(non_snake_case)]
pub mod android {

    use android_logger::{self, init_once};
    use jni::{objects::JClass, JNIEnv};
    use log::info;

    use crate::launch;

    // NOTE: Mind the `_1` to distingish the underscore _ from the underscore used to represent the dot in the java package name
    // com.erik_tesar.car.remote -> com_erik_*1*tesar_car_remote
    #[no_mangle]
    pub extern "system" fn Java_com_erik_1tesar_car_remote_RustService_startService(
        env: JNIEnv,
        _this: JClass,
    ) {
        init_once(
            android_logger::Config::default()
                .with_max_level(log::LevelFilter::Trace)
                .with_tag("RustApp"),
        );

        info!("Launching tokio...");
        match crate::log_error("launch returend an error", launch(env)) {
            Ok(_) => log::info!("Finished ok"),
            Err(e) => log::error!("Return error: {e:#?}",),
        };
    }
}

#[tokio::main(flavor = "current_thread")]
async fn launch(env: JNIEnv<'_>) -> color_eyre::Result<Infallible> {
    info!("Launched tokio!");
    let (ble_sender, search, listen, update, events) = ble::init(&env).await?;
    let sms = sms::init(ble_sender).await?;

    let e = tokio::select! {
        Err(e) = search => {
            error!("Search error: {e:#?}");
            e
        },
        Err(e) = listen => {
            error!("Listen error: {e:#?}");
            e

        },
        Err(e) = update => {
            error!("Update error: {e:#?}");
            e
        },
        Err(e) = sms => {
            error!("SMS error: {e:#?}");
            e
        },
        Err(e) = events => {
            error!("Event error: {e:#?}");
            e
        },
    };
    Err(e.into())
}

pub fn log_error<T>(
    msg: &str,
    res: color_eyre::Result<T>,
) -> color_eyre::Result<T> {
    match res {
        Ok(t) => Ok(t),
        Err(e) => {
            error!("{msg}: {e:#?}");
            Err(e)
        }
    }
}
