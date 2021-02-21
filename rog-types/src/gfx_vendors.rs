#[derive(Debug, PartialEq, Clone)]
pub enum GfxVendors {
    Nvidia,
    Integrated,
    Compute,
    Hybrid,
}

use std::str::FromStr;

use crate::error::GraphicsError;

impl FromStr for GfxVendors {
    type Err = GraphicsError;

    fn from_str(s: &str) -> Result<Self, GraphicsError> {
        match s.to_lowercase().as_str() {
            "nvidia" => Ok(GfxVendors::Nvidia),
            "hybrid" => Ok(GfxVendors::Hybrid),
            "compute" => Ok(GfxVendors::Compute),
            "integrated" => Ok(GfxVendors::Integrated),
            "nvidia\n" => Ok(GfxVendors::Nvidia),
            "hybrid\n" => Ok(GfxVendors::Hybrid),
            "compute\n" => Ok(GfxVendors::Compute),
            "integrated\n" => Ok(GfxVendors::Integrated),
            _ => Err(GraphicsError::ParseVendor),
        }
    }
}

impl From<&GfxVendors> for &str {
    fn from(mode: &GfxVendors) -> Self {
        match mode {
            GfxVendors::Nvidia => "nvidia",
            GfxVendors::Hybrid => "hybrid",
            GfxVendors::Compute => "compute",
            GfxVendors::Integrated => "integrated",
        }
    }
}

#[derive(Debug)]
pub enum GfxCtrlAction {
    Reboot,
    RestartX,
    None,
}

impl FromStr for GfxCtrlAction {
    type Err = GraphicsError;

    fn from_str(s: &str) -> Result<Self, GraphicsError> {
        match s.to_lowercase().as_str() {
            "reboot" => Ok(GfxCtrlAction::Reboot),
            "restartx" => Ok(GfxCtrlAction::RestartX),
            "none" => Ok(GfxCtrlAction::None),
            _ => Err(GraphicsError::ParseVendor),
        }
    }
}

impl From<&GfxCtrlAction> for &str {
    fn from(mode: &GfxCtrlAction) -> Self {
        match mode {
            GfxCtrlAction::Reboot => "reboot",
            GfxCtrlAction::RestartX => "restartx",
            GfxCtrlAction::None => "none",
        }
    }
}

impl From<&GfxCtrlAction> for String {
    fn from(mode: &GfxCtrlAction) -> Self {
        match mode {
            GfxCtrlAction::Reboot => "reboot".into(),
            GfxCtrlAction::RestartX => "restartx".into(),
            GfxCtrlAction::None => "none".into(),
        }
    }
}

impl From<GfxCtrlAction> for String {
    fn from(mode: GfxCtrlAction) -> Self {
        (&mode).into()
    }
}