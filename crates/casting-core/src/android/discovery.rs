use jni::sys::{jobjectArray, jsize};
use jni::{
    JNIEnv,
    objects::{JClass, JObject, JString},
};
use log::info;

// 2. 搜索接口
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "C" fn Java_zju_bangdream_ktv_casting_RustEngine_searchDevices(
    mut env: JNIEnv,
    _class: JClass,
) -> jobjectArray {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let dlna_devices = rt
        .block_on(crate::discover_devices_core())
        .unwrap_or_else(|error| {
            log::error!(target: "DLNA_DISCOVERY", "搜索接口返回空列表：原生搜索失败={}；空列表不等于设备离线", error);
            Vec::new()
        });

    let cls = env
        .find_class("zju/bangdream/ktv/casting/DlnaDeviceItem")
        .unwrap();
    let array = env
        .new_object_array(dlna_devices.len() as jsize, &cls, JObject::null())
        .unwrap();
    for (i, d) in dlna_devices.iter().enumerate() {
        let name = env.new_string(&d.friendly_name).unwrap();
        let loc = env.new_string(&d.location).unwrap();
        let item = env
            .new_object(
                &cls,
                "(Ljava/lang/String;Ljava/lang/String;)V",
                &[(&name).into(), (&loc).into()],
            )
            .unwrap();
        env.set_object_array_element(&array, i as jsize, item)
            .unwrap();
    }
    array.into_raw()
}

// 13. 搜索接口：通过设备描述XML的URL直接获取设备（适用于WiFi不支持多播的场景）
// 例如：http://192.168.1.x:9958/bilibili/description.xml（B站小电视默认地址）
// 成功返回包含一个元素的设备数组，失败返回空数组
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "C" fn Java_zju_bangdream_ktv_casting_RustEngine_searchDeviceByUrl(
    mut env: JNIEnv,
    _class: JClass,
    url: JString,
) -> jobjectArray {
    let url_str: String = env.get_string(&url).unwrap().into();
    let rt = tokio::runtime::Runtime::new().unwrap();

    let cls = env
        .find_class("zju/bangdream/ktv/casting/DlnaDeviceItem")
        .unwrap();

    match rt.block_on(crate::discover_device_from_url_core(url_str)) {
        Ok(device) => {
            let array = env
                .new_object_array(1, &cls, JObject::null())
                .unwrap();
            let name = env.new_string(&device.friendly_name).unwrap();
            let loc = env.new_string(&device.location).unwrap();
            let item = env
                .new_object(
                    &cls,
                    "(Ljava/lang/String;Ljava/lang/String;)V",
                    &[(&name).into(), (&loc).into()],
                )
                .unwrap();
            env.set_object_array_element(&array, 0, item).unwrap();
            array.into_raw()
        }
        Err(e) => {
            info!("通过URL搜索设备失败: {}", e);
            env.new_object_array(0, &cls, JObject::null())
                .unwrap()
                .into_raw()
        }
    }
}
