use egui::Ui;
use rog_platform::platform::GpuMode;
use rog_platform::supported::SupportedFunctions;
use rog_profiles::Profile;

use crate::system_state::SystemState;

pub fn platform_profile(states: &mut SystemState, ui: &mut Ui) {
    ui.heading("Platform profile");

    let mut changed = false;
    let mut item = |p: Profile, ui: &mut Ui| {
        if ui
            .selectable_value(&mut states.profiles.current, p, format!("{p:?}"))
            .clicked()
        {
            changed = true;
        }
    };

    ui.horizontal_wrapped(|ui| {
        for a in &states.profiles.list {
            item(*a, ui);
        }
    });

    if changed {
        states
            .asus_dbus
            .proxies()
            .profile()
            .set_active_profile(states.profiles.current)
            .map_err(|err| {
                states.error = Some(err.to_string());
            })
            .ok();
    };
}

pub fn rog_bios_group(supported: &SupportedFunctions, states: &mut SystemState, ui: &mut Ui) {
    ui.heading("Bios options");

    let slider = egui::Slider::new(&mut states.power_state.charge_limit, 20..=100)
        .text("Charging limit")
        .step_by(1.0);
    if ui.add(slider).drag_released() {
        states
            .asus_dbus
            .proxies()
            .charge()
            .set_charge_control_end_threshold(states.power_state.charge_limit)
            .map_err(|err| {
                states.error = Some(err.to_string());
            })
            .ok();
    }

    if supported.rog_bios_ctrl.post_animation_sound
        && ui
            .add(egui::Checkbox::new(
                &mut states.bios.post_sound,
                "POST sound",
            ))
            .changed()
    {
        states
            .asus_dbus
            .proxies()
            .rog_bios()
            .set_post_animation_sound(states.bios.post_sound)
            .map_err(|err| {
                states.error = Some(err.to_string());
            })
            .ok();
    }

    if supported.rog_bios_ctrl.panel_overdrive
        && ui
            .add(egui::Checkbox::new(
                &mut states.bios.panel_overdrive,
                "Panel overdrive",
            ))
            .changed()
    {
        states
            .asus_dbus
            .proxies()
            .rog_bios()
            .set_panel_od(states.bios.panel_overdrive)
            .map_err(|err| {
                states.error = Some(err.to_string());
            })
            .ok();
    }

    if supported.rog_bios_ctrl.mini_led_mode
        && ui
            .add(egui::Checkbox::new(
                &mut states.bios.mini_led_mode,
                "MiniLED backlight",
            ))
            .changed()
    {
        states
            .asus_dbus
            .proxies()
            .rog_bios()
            .set_mini_led_mode(states.bios.mini_led_mode)
            .map_err(|err| {
                states.error = Some(err.to_string());
            })
            .ok();
    }

    if supported.rog_bios_ctrl.gpu_mux {
        let mut changed = false;
        let mut dedicated_gfx = states.bios.dedicated_gfx;

        let mut reboot_required = false;
        if let Ok(mode) = states.asus_dbus.proxies().rog_bios().gpu_mux_mode() {
            reboot_required = GpuMode::from(mode) != states.bios.dedicated_gfx;
        }

        ui.group(|ui| {
            ui.vertical(|ui| {
                ui.horizontal_wrapped(|ui| ui.label("GPU MUX mode"));
                ui.horizontal_wrapped(|ui| ui.label("NOTE: Value does not change until rebooted"));
                ui.horizontal_wrapped(|ui| {
                    changed = ui
                        .selectable_value(
                            &mut dedicated_gfx,
                            GpuMode::Discrete,
                            "Dedicated (Ultimate)",
                        )
                        .clicked()
                        || ui
                            .selectable_value(
                                &mut dedicated_gfx,
                                GpuMode::Optimus,
                                "Optimus (Hybrid)",
                            )
                            .clicked();
                });

                if reboot_required {
                    ui.horizontal_wrapped(|ui| ui.heading("REBOOT REQUIRED"));
                }
            });
        });

        if changed {
            states
                .asus_dbus
                .proxies()
                .rog_bios()
                .set_gpu_mux_mode(dedicated_gfx)
                .map_err(|err| {
                    states.error = Some(err.to_string());
                })
                .ok();
        }
    }
}
