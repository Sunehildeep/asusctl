#![deny(unused_must_use)]
/// Configuration loading, saving
pub mod config;
/// Control of anime matrix display
pub mod ctrl_anime;
/// Keyboard LED brightness control, RGB, and LED display modes
pub mod ctrl_aura;
/// Control ASUS bios function such as boot sound, Optimus/Dedicated gfx mode
pub mod ctrl_platform;
/// Control of battery charge level
pub mod ctrl_power;
/// Control platform profiles + fan-curves if available
pub mod ctrl_profiles;

/// Fetch all supported functions for the laptop
pub mod ctrl_supported;

pub mod error;

use std::future::Future;
use std::time::Duration;

use async_trait::async_trait;
use dmi_id::DMIID;
use futures_lite::stream::StreamExt;
use log::{debug, info, warn};
use logind_zbus::manager::ManagerProxy;
use tokio::time::sleep;
use zbus::zvariant::ObjectPath;
use zbus::{CacheProperties, Connection, SignalContext};

use crate::error::RogError;

const CONFIG_PATH_BASE: &str = "/etc/asusd/";

/// This macro adds a function which spawns an `inotify` task on the passed in
/// `Executor`.
///
/// The generated function is `watch_<name>()`. Self requires the following
/// methods to be available:
/// - `<name>() -> SomeValue`, functionally is a getter, but is allowed to have
///   side effects.
/// - `notify_<name>(SignalContext, SomeValue)`
///
/// In most cases if `SomeValue` is stored in a config then `<name>()` getter is
/// expected to update it. The getter should *never* write back to the path or
/// attribute that is being watched or an infinite loop will occur.
///
/// # Example
///
/// ```ignore
/// impl CtrlRogBios {
///     task_watch_item!(panel_od platform);
///     task_watch_item!(gpu_mux_mode platform);
/// }
/// ```
/// // TODO: this is kind of useless if it can't trigger some action
#[macro_export]
macro_rules! task_watch_item {
    ($name:ident $self_inner:ident) => {
        concat_idents::concat_idents!(fn_name = watch_, $name {
        async fn fn_name(
            &self,
            signal_ctxt: SignalContext<'static>,
        ) -> Result<(), RogError> {
            use zbus::export::futures_util::StreamExt;

            let ctrl = self.clone();
            concat_idents::concat_idents!(watch_fn = monitor_, $name {
                match self.$self_inner.watch_fn() {
                    Ok(watch) => {
                        tokio::spawn(async move {
                            let mut buffer = [0; 32];
                            watch.into_event_stream(&mut buffer).unwrap().for_each(|_| async {
                                if let Ok(value) = ctrl.$name(){
                                    concat_idents::concat_idents!(notif_fn = $name, _changed {
                                        Self::notif_fn(&ctrl, &signal_ctxt).await.ok();
                                    });
                                    if let Some(mut lock) = ctrl.config.try_lock() {
                                        lock.$name = value;
                                        lock.write();
                                    }
                                }
                            }).await;
                        });
                    }
                    Err(e) => info!("inotify watch failed: {}. You can ignore this if your device does not support the feature", e),
                }
            });
            Ok(())
        }
        });
    };
}

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn print_board_info() {
    let dmi = DMIID::new().unwrap_or_default();
    info!("Product family: {}", dmi.product_family);
    info!("Board name: {}", dmi.board_name);
}

#[async_trait]
pub trait Reloadable {
    async fn reload(&mut self) -> Result<(), RogError>;
}

#[async_trait]
pub trait ZbusRun {
    async fn add_to_server(self, server: &mut Connection);

    async fn add_to_server_helper(
        iface: impl zbus::Interface,
        path: &str,
        server: &mut Connection,
    ) {
        server
            .object_server()
            .at(&ObjectPath::from_str_unchecked(path), iface)
            .await
            .map_err(|err| {
                warn!("{}: add_to_server {}", path, err);
                err
            })
            .ok();
    }
}

/// Set up a task to run on the async executor
#[async_trait]
pub trait CtrlTask {
    fn zbus_path() -> &'static str;

    fn signal_context(connection: &Connection) -> Result<SignalContext<'static>, zbus::Error> {
        SignalContext::new(connection, Self::zbus_path())
    }

    /// Implement to set up various tasks that may be required, using the
    /// `Executor`. No blocking loops are allowed, or they must be run on a
    /// separate thread.
    async fn create_tasks(&self, signal: SignalContext<'static>) -> Result<(), RogError>;

    // /// Create a timed repeating task
    // async fn repeating_task(&self, millis: u64, mut task: impl FnMut() + Send +
    // 'static) {     use std::time::Duration;
    //     use tokio::time;
    //     let mut timer = time::interval(Duration::from_millis(millis));
    //     tokio::spawn(async move {
    //         timer.tick().await;
    //         task();
    //     });
    // }

    /// Free helper method to create tasks to run on: sleep, wake, shutdown,
    /// boot
    ///
    /// The closures can potentially block, so execution time should be the
    /// minimal possible such as save a variable.
    async fn create_sys_event_tasks<
        Fut1,
        Fut2,
        Fut3,
        Fut4,
        F1: Send + 'static,
        F2: Send + 'static,
        F3: Send + 'static,
        F4: Send + 'static,
    >(
        &self,
        mut on_prepare_for_sleep: F1,
        mut on_prepare_for_shutdown: F2,
        mut on_lid_change: F3,
        mut on_external_power_change: F4,
    ) where
        F1: FnMut(bool) -> Fut1,
        F2: FnMut(bool) -> Fut2,
        F3: FnMut(bool) -> Fut3,
        F4: FnMut(bool) -> Fut4,
        Fut1: Future<Output = ()> + Send,
        Fut2: Future<Output = ()> + Send,
        Fut3: Future<Output = ()> + Send,
        Fut4: Future<Output = ()> + Send,
    {
        let connection = Connection::system()
            .await
            .expect("Controller could not create dbus connection");

        let manager = ManagerProxy::builder(&connection)
            .cache_properties(CacheProperties::No)
            .build()
            .await
            .expect("Controller could not create ManagerProxy");

        tokio::spawn(async move {
            if let Ok(mut notif) = manager.receive_prepare_for_sleep().await {
                while let Some(event) = notif.next().await {
                    if let Ok(args) = event.args() {
                        debug!("Doing on_prepare_for_sleep({})", args.start);
                        on_prepare_for_sleep(args.start).await;
                    }
                }
            }

            if let Ok(mut notif) = manager.receive_prepare_for_shutdown().await {
                while let Some(event) = notif.next().await {
                    if let Ok(args) = event.args() {
                        debug!("Doing on_prepare_for_shutdown({})", args.start);
                        on_prepare_for_shutdown(args.start).await;
                    }
                }
            }
        });

        let manager = ManagerProxy::builder(&connection)
            .cache_properties(CacheProperties::No)
            .build()
            .await
            .expect("Controller could not create ManagerProxy");

        tokio::spawn(async move {
            let mut last_power = manager.on_external_power().await.unwrap_or_default();
            let mut last_lid = manager.lid_closed().await.unwrap_or_default();
            // need to loop on these as they don't emit signals
            loop {
                if let Ok(next) = manager.on_external_power().await {
                    if next != last_power {
                        last_power = next;
                        on_external_power_change(next).await;
                    }
                }
                if let Ok(next) = manager.lid_closed().await {
                    if next != last_lid {
                        last_lid = next;
                        on_lid_change(next).await;
                    }
                }
                sleep(Duration::from_secs(2)).await;
            }
        });
    }
}

pub trait GetSupported {
    type A;

    fn get_supported() -> Self::A;
}
