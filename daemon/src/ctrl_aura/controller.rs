// Only these two packets must be 17 bytes
static KBD_BRIGHT_PATH: &str = "/sys/class/leds/asus::kbd_backlight/brightness";

use crate::{
    error::RogError,
    laptops::{LaptopLedData, ASUS_KEYBOARD_DEVICES},
    CtrlTask,
};
use async_trait::async_trait;
use log::{error, info, warn};
use logind_zbus::manager::ManagerProxy;
use rog_aura::usb::leds_message;
use rog_aura::{
    usb::{LED_APPLY, LED_SET},
    AuraEffect, LedBrightness, LED_MSG_LEN,
};
use rog_supported::LedSupportedFunctions;
use smol::{stream::StreamExt, Executor};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::Arc;
use std::sync::Mutex;
use std::{fs::OpenOptions, sync::MutexGuard};
use zbus::Connection;

use crate::GetSupported;

use super::config::AuraConfig;

impl GetSupported for CtrlKbdLed {
    type A = LedSupportedFunctions;

    fn get_supported() -> Self::A {
        // let mode = <&str>::from(&<AuraModes>::from(*mode));
        let laptop = LaptopLedData::get_data();
        let stock_led_modes = laptop.standard;
        let multizone_led_mode = laptop.multizone;
        let per_key_led_mode = laptop.per_key;

        LedSupportedFunctions {
            brightness_set: CtrlKbdLed::get_kbd_bright_path().is_some(),
            stock_led_modes,
            multizone_led_mode,
            per_key_led_mode,
        }
    }
}

pub struct CtrlKbdLed {
    pub led_node: Option<String>,
    pub bright_node: String,
    pub supported_modes: LaptopLedData,
    pub flip_effect_write: bool,
    pub config: AuraConfig,
}

pub struct CtrlKbdLedTask {
    inner: Arc<Mutex<CtrlKbdLed>>,
}

impl CtrlKbdLedTask {
    pub fn new(inner: Arc<Mutex<CtrlKbdLed>>) -> Self {
        Self { inner }
    }

    fn update_config(lock: &mut CtrlKbdLed) -> Result<(), RogError> {
        let mut file = OpenOptions::new()
            .read(true)
            .open(&lock.bright_node)
            .map_err(|err| match err.kind() {
                std::io::ErrorKind::NotFound => {
                    RogError::MissingLedBrightNode((&lock.bright_node).into(), err)
                }
                _ => RogError::Path((&lock.bright_node).into(), err),
            })?;
        let mut buf = [0u8; 1];
        file.read_exact(&mut buf)
            .map_err(|err| RogError::Read("buffer".into(), err))?;
        if let Some(num) = char::from(buf[0]).to_digit(10) {
            if lock.config.brightness != num.into() {
                lock.config.read();
                lock.config.brightness = num.into();
                lock.config.write();
            }
            return Ok(());
        }
        Err(RogError::ParseLed)
    }
}

#[async_trait]
impl CtrlTask for CtrlKbdLedTask {
    async fn create_tasks(&self, executor: &mut Executor) -> Result<(), RogError> {
        let connection = Connection::system()
            .await
            .expect("CtrlKbdLedTask could not create dbus connection");

        let manager = ManagerProxy::new(&connection)
            .await
            .expect("CtrlKbdLedTask could not create ManagerProxy");

        let load_save = |start: bool, mut lock: MutexGuard<CtrlKbdLed>| {
            // If waking up
            if !start {
                info!("CtrlKbdLedTask reloading brightness and modes");
                lock.set_brightness(lock.config.brightness)
                    .map_err(|e| error!("CtrlKbdLedTask: {e}"))
                    .ok();
                if let Some(mode) = lock.config.builtins.get(&lock.config.current_mode) {
                    lock.write_mode(mode)
                        .map_err(|e| error!("CtrlKbdLedTask: {e}"))
                        .ok();
                }
            } else if start {
                info!("CtrlKbdLedTask saving last brightness");
                Self::update_config(&mut lock)
                    .map_err(|e| error!("CtrlKbdLedTask: {e}"))
                    .ok();
            }
        };

        let inner = self.inner.clone();
        executor
            .spawn(async move {
                if let Ok(notif) = manager.receive_prepare_for_sleep().await {
                    notif
                        .for_each(|event| {
                            if let Ok(args) = event.args() {
                                loop {
                                    // Loop so that we do aquire the lock but also don't block other
                                    // threads (prevents potential deadlocks)
                                    if let Ok(lock) = inner.clone().try_lock() {
                                        load_save(args.start, lock);
                                        break;
                                    }
                                }
                            }
                        })
                        .await;
                }
                if let Ok(notif) = manager.receive_prepare_for_shutdown().await {
                    notif
                        .for_each(|event| {
                            if let Ok(args) = event.args() {
                                loop {
                                    if let Ok(lock) = inner.clone().try_lock() {
                                        load_save(args.start, lock);
                                        break;
                                    }
                                }
                            }
                        })
                        .await;
                }
            })
            .detach();

        // let inner = self.inner.clone();
        // self.repeating_task(500, executor, move || loop {
        //     if let Ok(ref mut lock) = inner.try_lock() {
        //         Self::update_config(lock).unwrap();
        //         break;
        //     }
        // })
        // .await;
        Ok(())
    }
}

pub struct CtrlKbdLedReloader(pub Arc<Mutex<CtrlKbdLed>>);

impl crate::Reloadable for CtrlKbdLedReloader {
    fn reload(&mut self) -> Result<(), RogError> {
        if let Ok(mut ctrl) = self.0.try_lock() {
            let current = ctrl.config.current_mode;
            if let Some(mode) = ctrl.config.builtins.get(&current).cloned() {
                ctrl.do_command(mode).ok();
            }

            ctrl.set_power_states(&ctrl.config)
                .map_err(|err| warn!("{err}"))
                .ok();
        }
        Ok(())
    }
}

pub struct CtrlKbdLedZbus(pub Arc<Mutex<CtrlKbdLed>>);

impl CtrlKbdLedZbus {
    pub fn new(inner: Arc<Mutex<CtrlKbdLed>>) -> Self {
        Self(inner)
    }
}

impl CtrlKbdLed {
    #[inline]
    pub fn new(supported_modes: LaptopLedData, config: AuraConfig) -> Result<Self, RogError> {
        // TODO: return error if *all* nodes are None
        let mut led_node = None;
        for prod in ASUS_KEYBOARD_DEVICES.iter() {
            match Self::find_led_node(prod) {
                Ok(node) => {
                    led_node = Some(node);
                    info!("Looked for keyboard controller 0x{prod}: Found");
                    break;
                }
                Err(err) => info!("Looked for keyboard controller 0x{prod}: {err}"),
            }
        }

        let bright_node = Self::get_kbd_bright_path();

        if led_node.is_none() && bright_node.is_none() {
            return Err(RogError::MissingFunction(
                "All keyboard features missing, you may require a v5.11 series kernel or newer"
                    .into(),
            ));
        }

        if bright_node.is_none() {
            return Err(RogError::MissingFunction(
                "No brightness control, you may require a v5.11 series kernel or newer".into(),
            ));
        }

        let ctrl = CtrlKbdLed {
            led_node,
            bright_node: bright_node.unwrap(), // If was none then we already returned above
            supported_modes,
            flip_effect_write: false,
            config,
        };
        Ok(ctrl)
    }

    fn get_kbd_bright_path() -> Option<String> {
        if Path::new(KBD_BRIGHT_PATH).exists() {
            return Some(KBD_BRIGHT_PATH.to_string());
        }
        None
    }

    pub(super) fn get_brightness(&self) -> Result<u8, RogError> {
        let mut file = OpenOptions::new()
            .read(true)
            .open(&self.bright_node)
            .map_err(|err| match err.kind() {
                std::io::ErrorKind::NotFound => {
                    RogError::MissingLedBrightNode((&self.bright_node).into(), err)
                }
                _ => RogError::Path((&self.bright_node).into(), err),
            })?;
        let mut buf = [0u8; 1];
        file.read_exact(&mut buf)
            .map_err(|err| RogError::Read("buffer".into(), err))?;
        Ok(buf[0])
    }

    pub(super) fn set_brightness(&self, brightness: LedBrightness) -> Result<(), RogError> {
        let path = Path::new(&self.bright_node);
        let mut file =
            OpenOptions::new()
                .write(true)
                .open(&path)
                .map_err(|err| match err.kind() {
                    std::io::ErrorKind::NotFound => {
                        RogError::MissingLedBrightNode((&self.bright_node).into(), err)
                    }
                    _ => RogError::Path((&self.bright_node).into(), err),
                })?;
        file.write_all(&[brightness.as_char_code()])
            .map_err(|err| RogError::Read("buffer".into(), err))?;
        Ok(())
    }

    pub fn next_brightness(&mut self) -> Result<(), RogError> {
        let mut bright = (self.config.brightness as u32) + 1;
        if bright > 3 {
            bright = 0;
        }
        self.config.brightness = <LedBrightness>::from(bright);
        self.config.write();
        self.set_brightness(self.config.brightness)
    }

    pub fn prev_brightness(&mut self) -> Result<(), RogError> {
        let mut bright = self.config.brightness as u32;
        if bright == 0 {
            bright = 3;
        } else {
            bright -= 1;
        }
        self.config.brightness = <LedBrightness>::from(bright);
        self.config.write();
        self.set_brightness(self.config.brightness)
    }

    /// Set combination state for boot animation/sleep animation/all leds/keys leds/side leds LED active
    pub(super) fn set_power_states(&self, config: &AuraConfig) -> Result<(), RogError> {
        let bytes = leds_message(
            config.power_states.boot_anim,
            config.power_states.sleep_anim,
            config.power_states.all_leds,
            config.power_states.keys_leds,
            config.power_states.side_leds,
        );

        // Quite ugly, must be a more idiomatic way to do
        let message = [
            0x5d, 0xbd, 0x01, bytes[0], bytes[1], bytes[2], 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ];

        self.write_bytes(&message)?;
        self.write_bytes(&LED_SET)?;
        // Changes won't persist unless apply is set
        self.write_bytes(&LED_APPLY)?;
        Ok(())
    }

    fn find_led_node(id_product: &str) -> Result<String, RogError> {
        let mut enumerator = udev::Enumerator::new().map_err(|err| {
            warn!("{}", err);
            RogError::Udev("enumerator failed".into(), err)
        })?;
        enumerator.match_subsystem("hidraw").map_err(|err| {
            warn!("{}", err);
            RogError::Udev("match_subsystem failed".into(), err)
        })?;

        for device in enumerator.scan_devices().map_err(|err| {
            warn!("{}", err);
            RogError::Udev("scan_devices failed".into(), err)
        })? {
            if let Some(parent) = device
                .parent_with_subsystem_devtype("usb", "usb_device")
                .map_err(|err| {
                    warn!("{}", err);
                    RogError::Udev("parent_with_subsystem_devtype failed".into(), err)
                })?
            {
                if parent
                    .attribute_value("idProduct")
                    .ok_or_else(|| RogError::NotFound("LED idProduct".into()))?
                    == id_product
                {
                    if let Some(dev_node) = device.devnode() {
                        info!("Using device at: {:?} for LED control", dev_node);
                        return Ok(dev_node.to_string_lossy().to_string());
                    }
                }
            }
        }
        Err(RogError::MissingFunction(
            "ASUS LED device node not found".into(),
        ))
    }

    pub(crate) fn do_command(&mut self, mode: AuraEffect) -> Result<(), RogError> {
        self.set_and_save(mode)
    }

    /// Should only be used if the bytes you are writing are verified correct
    #[inline]
    fn write_bytes(&self, message: &[u8]) -> Result<(), RogError> {
        if let Some(led_node) = &self.led_node {
            if let Ok(mut file) = OpenOptions::new().write(true).open(led_node) {
                // println!("write: {:02x?}", &message);
                return file
                    .write_all(message)
                    .map_err(|err| RogError::Write("write_bytes".into(), err));
            }
        }
        Err(RogError::NotSupported)
    }

    /// Write an effect block
    #[inline]
    fn _write_effect(&mut self, effect: &[Vec<u8>]) -> Result<(), RogError> {
        if self.flip_effect_write {
            for row in effect.iter().rev() {
                self.write_bytes(row)?;
            }
        } else {
            for row in effect.iter() {
                self.write_bytes(row)?;
            }
        }
        self.flip_effect_write = !self.flip_effect_write;
        Ok(())
    }

    /// Used to set a builtin mode and save the settings for it
    ///
    /// This needs to be universal so that settings applied by dbus stick
    #[inline]
    fn set_and_save(&mut self, mode: AuraEffect) -> Result<(), RogError> {
        self.config.read();
        self.write_mode(&mode)?;
        self.config.current_mode = *mode.mode();
        self.config.set_builtin(mode);
        self.config.write();
        Ok(())
    }

    #[inline]
    pub(super) fn toggle_mode(&mut self, reverse: bool) -> Result<(), RogError> {
        let current = self.config.current_mode;
        if let Some(idx) = self
            .supported_modes
            .standard
            .iter()
            .position(|v| *v == current)
        {
            let mut idx = idx;
            // goes past end of array
            if reverse {
                if idx == 0 {
                    idx = self.supported_modes.standard.len() - 1;
                } else {
                    idx -= 1;
                }
            } else {
                idx += 1;
                if idx == self.supported_modes.standard.len() {
                    idx = 0;
                }
            }
            let next = self.supported_modes.standard[idx];

            self.config.read();
            if let Some(data) = self.config.builtins.get(&next) {
                self.write_mode(data)?;
                self.config.current_mode = next;
            }
            self.config.write();
        }

        Ok(())
    }

    #[inline]
    fn write_mode(&self, mode: &AuraEffect) -> Result<(), RogError> {
        if !self.supported_modes.standard.contains(mode.mode()) {
            return Err(RogError::NotSupported);
        }
        let bytes: [u8; LED_MSG_LEN] = mode.into();
        self.write_bytes(&bytes)?;
        self.write_bytes(&LED_SET)?;
        // Changes won't persist unless apply is set
        self.write_bytes(&LED_APPLY)?;
        Ok(())
    }
}
