use super::DisplayMetadata;
use crate::error::{CommandError, CommandResult};
use std::{collections::HashMap, mem::size_of, ptr};
use tauri::window::Monitor;
use windows_sys::Win32::{
    Devices::Display::*,
    Foundation::{ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS},
};

pub(super) fn metadata(monitors: &[Monitor]) -> CommandResult<Vec<Option<DisplayMetadata>>> {
    let names = friendly_names().map_err(|error| {
        CommandError::new("DISPLAY_DISCOVERY", format!("读取显示器名称失败：{error}"))
    })?;
    Ok(monitors
        .iter()
        .map(|monitor| {
            let source = monitor.name()?.to_ascii_lowercase();
            names.get(&source).map(|name| DisplayMetadata {
                name: name.clone(),
                built_in: None,
            })
        })
        .collect())
}

fn wide_string(value: &[u16]) -> String {
    let end = value
        .iter()
        .position(|&unit| unit == 0)
        .unwrap_or(value.len());
    String::from_utf16_lossy(&value[..end]).trim().to_owned()
}

/// 通过活动显示路径，把 GDI 输出标识与对应显示器的名称关联。
fn friendly_names() -> Result<HashMap<String, String>, u32> {
    for _ in 0..3 {
        let mut path_count = 0;
        let mut mode_count = 0;
        // API 仅写入计数及下方按计数分配的缓冲区。
        let status = unsafe {
            GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut path_count, &mut mode_count)
        };
        if status != ERROR_SUCCESS {
            return Err(status);
        }
        let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); path_count as usize];
        let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); mode_count as usize];
        let status = unsafe {
            QueryDisplayConfig(
                QDC_ONLY_ACTIVE_PATHS,
                &mut path_count,
                paths.as_mut_ptr(),
                &mut mode_count,
                modes.as_mut_ptr(),
                ptr::null_mut(),
            )
        };
        // 查询期间插拔屏幕可能使第一次取得的缓冲区大小失效。
        if status == ERROR_INSUFFICIENT_BUFFER {
            continue;
        }
        if status != ERROR_SUCCESS {
            return Err(status);
        }
        let mut names: HashMap<String, Vec<(String, String)>> = HashMap::new();
        for path in paths.iter().take(path_count as usize) {
            let mut source = DISPLAYCONFIG_SOURCE_DEVICE_NAME {
                header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
                    r#type: DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
                    size: size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32,
                    adapterId: path.sourceInfo.adapterId,
                    id: path.sourceInfo.id,
                },
                ..Default::default()
            };
            let mut target = DISPLAYCONFIG_TARGET_DEVICE_NAME {
                header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
                    r#type: DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
                    size: size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>() as u32,
                    adapterId: path.targetInfo.adapterId,
                    id: path.targetInfo.id,
                },
                ..Default::default()
            };
            // 两个请求均包含与结构体大小、设备类型对应的有效头部。
            if unsafe { DisplayConfigGetDeviceInfo(&mut source.header) } != 0
                || unsafe { DisplayConfigGetDeviceInfo(&mut target.header) } != 0
            {
                continue;
            }
            let source_name = wide_string(&source.viewGdiDeviceName);
            let target_name = wide_string(&target.monitorFriendlyDeviceName);
            let device_path = wide_string(&target.monitorDevicePath).to_ascii_lowercase();
            if source_name.is_empty() || device_path.is_empty() {
                continue;
            }
            let entries = names.entry(source_name.to_ascii_lowercase()).or_default();
            entries.push((device_path, target_name));
        }
        // 按设备路径排序，取当前 GDI 输出对应的首个目标名称。
        return Ok(names
            .into_iter()
            .map(|(source, mut targets)| {
                targets.sort_by(|left, right| left.0.cmp(&right.0));
                (source, targets.remove(0).1)
            })
            .collect());
    }
    Err(ERROR_INSUFFICIENT_BUFFER)
}
