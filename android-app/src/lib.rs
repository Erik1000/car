use std::sync::OnceLock;

use jni::{JNIEnv, JavaVM};
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

        // let res: jint = 4;
        // //env.call_method(callback, "factCallback", "(I)V", &[res.into()])
        // match env.call_method(this, "factCallback", "(I)V", &[res.into()]) {
        //     Ok(_) => info!("Worked!"),
        //     Err(e) => error!("Failed: {e:?}"),
        // }
    }
}

pub static JAVA_VM: OnceLock<JavaVM> = OnceLock::new();

#[tokio::main(flavor = "current_thread")]
async fn launch(env: JNIEnv<'_>) -> color_eyre::Result<()> {
    info!("Launched tokio!");
    let (ble_sender, search, listen, update, events) = ble::init(&env).await?;
    let sms = sms::init(ble_sender).await?;
    let (search, listen, update, sms, events) =
    // FIXME: should be select to catch early returns e.g. because of panic
        tokio::join!(search, listen, update, sms, events);
    search??;
    listen??;
    update??;
    sms??;
    events??;
    Ok(())
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
