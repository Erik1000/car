mod service;

#[allow(non_snake_case)]
pub mod android {
    use std::thread::spawn;

    use android_logger::{self, init_once};
    use jni::{
        objects::{JClass, JString},
        JNIEnv,
    };

    use crate::service::{launch, watch};

    // NOTE: Mind the `_1` to distingish the underscore _ from the underscore used to represent the dot in the java package name
    // com.erik_tesar.car.remote -> com_erik_*1*tesar_car_remote
    #[no_mangle]
    pub extern "system" fn Java_com_erik_1tesar_car_remote_RustService_startService(
        _: JNIEnv,
        _: JClass,
        _: JString,
    ) {
        init_once(
            android_logger::Config::default()
                .with_max_level(log::LevelFilter::Trace)
                .with_tag("RustApp"),
        );

        spawn(|| {
            watch(
                "/data/user/0/com.erik_tesar.car.remote/files/fsmon_log.yaml",
                vec!["/storage/emulated/0/Documents"],
            );
        });

        spawn(|| {
            launch(
                "/data/user/0/com.erik_tesar.car.remote/files/fsmon_log.yaml",
            );
        });
    }
}
