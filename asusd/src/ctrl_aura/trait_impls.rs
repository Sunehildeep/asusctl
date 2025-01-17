use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use config_traits::StdConfig;
use log::{debug, error, info, warn};
use rog_aura::advanced::UsbPackets;
use rog_aura::usb::{AuraDevice, AuraPowerDev};
use rog_aura::{AuraEffect, AuraModeNum, LedBrightness};
use zbus::export::futures_util::lock::{Mutex, MutexGuard};
use zbus::export::futures_util::StreamExt;
use zbus::{dbus_interface, Connection, SignalContext};

use super::controller::CtrlKbdLed;
use crate::error::RogError;
use crate::CtrlTask;

pub(super) const ZBUS_PATH: &str = "/org/asuslinux/Aura";

#[derive(Clone)]
pub struct CtrlKbdLedZbus(pub Arc<Mutex<CtrlKbdLed>>);

impl CtrlKbdLedZbus {
    fn update_config(lock: &mut CtrlKbdLed) -> Result<(), RogError> {
        let bright = lock.kd_brightness.get_brightness()?;
        lock.config.read();
        lock.config.brightness = (bright as u32).into();
        lock.config.write();
        Ok(())
    }
}

#[async_trait]
impl crate::ZbusRun for CtrlKbdLedZbus {
    async fn add_to_server(self, server: &mut Connection) {
        Self::add_to_server_helper(self, ZBUS_PATH, server).await;
    }
}

/// The main interface for changing, reading, or notfying signals
///
/// LED commands are split between Brightness, Modes, Per-Key
#[dbus_interface(name = "org.asuslinux.Daemon")]
impl CtrlKbdLedZbus {
    /// Set the keyboard brightness level (0-3)
    async fn set_brightness(&mut self, brightness: LedBrightness) {
        let ctrl = self.0.lock().await;
        ctrl.set_brightness(brightness)
            .map_err(|err| warn!("{}", err))
            .ok();
    }

    /// Set a variety of states, input is array of enum.
    /// `enabled` sets if the sent array should be disabled or enabled
    ///
    /// For Modern ROG devices the "enabled" flag is ignored.
    async fn set_led_power(
        &mut self,
        #[zbus(signal_context)] ctxt: SignalContext<'_>,
        options: AuraPowerDev,
        enabled: bool,
    ) -> zbus::fdo::Result<()> {
        let mut ctrl = self.0.lock().await;
        for p in options.tuf {
            ctrl.config.enabled.set_tuf(p, enabled);
        }
        for p in options.old_rog {
            ctrl.config.enabled.set_0x1866(p, enabled);
        }
        ctrl.config.enabled.set_0x19b6(options.rog);

        ctrl.config.write();

        ctrl.set_power_states().map_err(|e| {
            warn!("{}", e);
            e
        })?;

        Self::notify_power_states(&ctxt, &AuraPowerDev::from(&ctrl.config.enabled))
            .await
            .unwrap_or_else(|err| warn!("{}", err));
        Ok(())
    }

    async fn set_led_mode(
        &mut self,
        #[zbus(signal_context)] ctxt: SignalContext<'_>,
        effect: AuraEffect,
    ) -> zbus::fdo::Result<()> {
        let mut ctrl = self.0.lock().await;

        ctrl.set_effect(effect).map_err(|e| {
            warn!("{}", e);
            e
        })?;

        ctrl.set_brightness(ctrl.config.brightness).map_err(|e| {
            warn!("{}", e);
            e
        })?;

        if let Some(mode) = ctrl.config.builtins.get(&ctrl.config.current_mode) {
            Self::notify_led(&ctxt, mode.clone())
                .await
                .unwrap_or_else(|err| warn!("{}", err));
        }
        Ok(())
    }

    async fn next_led_mode(
        &self,
        #[zbus(signal_context)] ctxt: SignalContext<'_>,
    ) -> zbus::fdo::Result<()> {
        let mut ctrl = self.0.lock().await;

        ctrl.toggle_mode(false).map_err(|e| {
            warn!("{}", e);
            e
        })?;

        if let Some(mode) = ctrl.config.builtins.get(&ctrl.config.current_mode) {
            Self::notify_led(&ctxt, mode.clone())
                .await
                .unwrap_or_else(|err| warn!("{}", err));
        }

        Ok(())
    }

    async fn prev_led_mode(
        &self,
        #[zbus(signal_context)] ctxt: SignalContext<'_>,
    ) -> zbus::fdo::Result<()> {
        let mut ctrl = self.0.lock().await;

        ctrl.toggle_mode(true).map_err(|e| {
            warn!("{}", e);
            e
        })?;

        if let Some(mode) = ctrl.config.builtins.get(&ctrl.config.current_mode) {
            Self::notify_led(&ctxt, mode.clone())
                .await
                .unwrap_or_else(|err| warn!("{}", err));
        }

        Ok(())
    }

    async fn next_led_brightness(&self) -> zbus::fdo::Result<()> {
        let mut ctrl = self.0.lock().await;
        ctrl.next_brightness().map_err(|e| {
            warn!("{}", e);
            e
        })?;
        Ok(())
    }

    async fn prev_led_brightness(&self) -> zbus::fdo::Result<()> {
        let mut ctrl = self.0.lock().await;
        ctrl.prev_brightness().map_err(|e| {
            warn!("{}", e);
            e
        })?;
        Ok(())
    }

    /// Return the device type for this Aura keyboard
    async fn device_type(&self) -> AuraDevice {
        let ctrl = self.0.lock().await;
        ctrl.led_prod
    }

    // As property doesn't work for AuraPowerDev (complexity of serialization?)
    // #[dbus_interface(property)]
    async fn led_power(&self) -> AuraPowerDev {
        let ctrl = self.0.lock().await;
        AuraPowerDev::from(&ctrl.config.enabled)
    }

    /// Return the current mode data
    async fn led_mode(&self) -> AuraModeNum {
        let ctrl = self.0.lock().await;
        ctrl.config.current_mode
    }

    /// Return a list of available modes
    async fn led_modes(&self) -> BTreeMap<AuraModeNum, AuraEffect> {
        let ctrl = self.0.lock().await;
        ctrl.config.builtins.clone()
    }

    /// On machine that have some form of either per-key keyboard or per-zone
    /// this can be used to write custom effects over dbus. The input is a
    /// nested `Vec<Vec<8>>` where `Vec<u8>` is a raw USB packet
    async fn direct_addressing_raw(&self, data: UsbPackets) -> zbus::fdo::Result<()> {
        let mut ctrl = self.0.lock().await;
        ctrl.write_effect_block(&data)?;
        Ok(())
    }

    /// Return the current LED brightness
    #[dbus_interface(property)]
    async fn led_brightness(&self) -> i8 {
        let ctrl = self.0.lock().await;
        ctrl.get_brightness().map(|n| n as i8).unwrap_or(-1)
    }

    #[dbus_interface(signal)]
    async fn notify_led(signal_ctxt: &SignalContext<'_>, data: AuraEffect) -> zbus::Result<()>;

    #[dbus_interface(signal)]
    async fn notify_power_states(
        signal_ctxt: &SignalContext<'_>,
        data: &AuraPowerDev,
    ) -> zbus::Result<()>;
}

#[async_trait]
impl CtrlTask for CtrlKbdLedZbus {
    fn zbus_path() -> &'static str {
        ZBUS_PATH
    }

    async fn create_tasks(&self, _: SignalContext<'static>) -> Result<(), RogError> {
        let load_save = |start: bool, mut lock: MutexGuard<'_, CtrlKbdLed>| {
            // If waking up
            if !start {
                info!("CtrlKbdLedTask reloading brightness and modes");
                lock.set_brightness(lock.config.brightness)
                    .map_err(|e| error!("CtrlKbdLedTask: {e}"))
                    .ok();
                lock.write_current_config_mode()
                    .map_err(|e| error!("CtrlKbdLedTask: {e}"))
                    .ok();
            } else if start {
                Self::update_config(&mut lock)
                    .map_err(|e| error!("CtrlKbdLedTask: {e}"))
                    .ok();
            }
        };

        let inner1 = self.0.clone();
        let inner3 = self.0.clone();
        self.create_sys_event_tasks(
            move |sleeping| {
                let inner1 = inner1.clone();
                async move {
                    let lock = inner1.lock().await;
                    load_save(sleeping, lock);
                }
            },
            move |_shutting_down| {
                let inner3 = inner3.clone();
                async move {
                    let lock = inner3.lock().await;
                    load_save(false, lock);
                }
            },
            move |_lid_closed| {
                // on lid change
                async move {}
            },
            move |_power_plugged| {
                // power change
                async move {}
            },
        )
        .await;

        let ctrl2 = self.0.clone();
        let ctrl = self.0.lock().await;
        let watch = ctrl.kd_brightness.monitor_brightness()?;
        tokio::spawn(async move {
            let mut buffer = [0; 32];
            watch
                .into_event_stream(&mut buffer)
                .unwrap()
                .for_each(|_| async {
                    if let Some(lock) = ctrl2.try_lock() {
                        load_save(true, lock);
                    }
                })
                .await;
        });

        Ok(())
    }
}

#[async_trait]
impl crate::Reloadable for CtrlKbdLedZbus {
    async fn reload(&mut self) -> Result<(), RogError> {
        let mut ctrl = self.0.lock().await;
        debug!("CtrlKbdLedZbus: reloading keyboard mode");
        ctrl.write_current_config_mode()?;
        debug!("CtrlKbdLedZbus: reloading power states");
        ctrl.set_power_states().map_err(|err| warn!("{err}")).ok();
        Ok(())
    }
}
