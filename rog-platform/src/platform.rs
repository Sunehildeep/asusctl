use std::path::PathBuf;

use log::warn;
use serde::{Deserialize, Serialize};
use zvariant::Type;

use crate::{
    attr_bool, attr_u8,
    error::{PlatformError, Result},
    to_device,
};

/// The "platform" device provides access to things like:
/// - dgpu_disable
/// - egpu_enable
/// - panel_od
/// - dgpu_only
/// - keyboard_mode, set keyboard RGB mode and speed
/// - keyboard_state, set keyboard power states
#[derive(Debug, PartialEq, PartialOrd)]
pub struct AsusPlatform(PathBuf);

impl AsusPlatform {
    pub fn new() -> Result<Self> {
        let mut enumerator = udev::Enumerator::new().map_err(|err| {
            warn!("{}", err);
            PlatformError::Udev("enumerator failed".into(), err)
        })?;
        enumerator.match_subsystem("platform").map_err(|err| {
            warn!("{}", err);
            PlatformError::Udev("match_subsystem failed".into(), err)
        })?;
        enumerator.match_sysname("asus-nb-wmi").map_err(|err| {
            warn!("{}", err);
            PlatformError::Udev("match_subsystem failed".into(), err)
        })?;

        for device in enumerator.scan_devices().map_err(|err| {
            warn!("{}", err);
            PlatformError::Udev("scan_devices failed".into(), err)
        })? {
            return Ok(Self(device.syspath().to_owned()));
        }
        Err(PlatformError::MissingFunction(
            "asus-nb-wmi not found".into(),
        ))
    }

    attr_bool!(
        has_dgpu_disable,
        get_dgpu_disable,
        set_dgpu_disable,
        "dgpu_disable"
    );

    attr_bool!(
        has_egpu_enable,
        get_egpu_enable,
        set_egpu_enable,
        "egpu_enable"
    );

    attr_bool!(has_panel_od, get_panel_od, set_panel_od, "panel_od");

    attr_u8!(
        has_gpu_mux_mode,
        get_gpu_mux_mode,
        set_gpu_mux_mode,
        "gpu_mux_mode"
    );
}

#[derive(Serialize, Deserialize, Type, Debug, PartialEq, Clone, Copy)]
pub enum GpuMuxMode {
    Discrete,
    Optimus,
    Error,
    NotSupported,
}

impl From<u8> for GpuMuxMode {
    fn from(m: u8) -> Self {
        if m > 0 {
            return Self::Optimus;
        }
        Self::Discrete
    }
}

impl From<GpuMuxMode> for u8 {
    fn from(m: GpuMuxMode) -> Self {
        match m {
            GpuMuxMode::Discrete => 0,
            GpuMuxMode::Optimus => 1,
            GpuMuxMode::Error => 254,
            GpuMuxMode::NotSupported => 255,
        }
    }
}
