use crate::error::{CommandError, CommandResult};
use serde::Deserialize;
use tauri::{Manager, plugin::PluginHandle};

struct AndroidNetwork<R: tauri::Runtime>(PluginHandle<R>);

pub fn init<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri::plugin::Builder::new("casting-network")
        .setup(|app, api| {
            let handle = api.register_android_plugin("zju.idol.karamas", "CastingNetworkPlugin")?;
            app.manage(AndroidNetwork(handle));
            Ok(())
        })
        .build()
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct NetworkInfo {
    wifi: bool,
    vpn: bool,
    interface_name: Option<String>,
    addresses: Vec<String>,
}

pub async fn ensure_network_access<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> CommandResult<()> {
    let network = app.state::<AndroidNetwork<R>>();
    let info: NetworkInfo = network
        .0
        .run_mobile_plugin_async("ensureAccess", ())
        .await
        .map_err(|error| CommandError::new("ANDROID_NETWORK", error.to_string()))?;
    log::info!(
        target: "ANDROID_NETWORK",
        "默认网络：Wi-Fi={}，VPN={}，接口={:?}，地址={:?}",
        info.wifi, info.vpn, info.interface_name, info.addresses
    );
    Ok(())
}
