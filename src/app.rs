use egui::Align2;
use egui::Color32;
use egui::FontId;
use egui::Pos2;
use egui_overlay::EguiOverlay;
use egui_overlay::egui_window_glfw_passthrough;
use crate::driver::DriverManager;
use dumpspace_api::DSAPI;

#[cfg(feature = "three_d")]
use egui_render_three_d::ThreeDBackend as DefaultGfxBackend;
#[cfg(feature = "wgpu")]
use egui_render_wgpu::WgpuBackend as DefaultGfxBackend;

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct FVector {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

pub struct App {
    pub frame: u64,
    pub fullscreen_applied: bool,
    pub driver: DriverManager,
    pub dsapi: DSAPI,
    pub base_address: u64,
    pub uworld: Option<u64>,
    pub player_location: Option<FVector>,
    pub last_frame_time: std::time::Instant,
    pub fps: f32,
}

impl EguiOverlay for App {
    fn gui_run(
        &mut self,
        egui_context: &egui::Context,
        _default_gfx_backend: &mut DefaultGfxBackend,
        glfw_backend: &mut egui_window_glfw_passthrough::GlfwBackend,
    ) {
        if !self.fullscreen_applied {
            glfw_backend.window.set_decorated(false);
            glfw_backend.window.maximize();
            glfw_backend.window.set_pos(0, 0);
            self.fullscreen_applied = true;
        }

        // Calculate FPS
        let now = std::time::Instant::now();
        let delta = now.duration_since(self.last_frame_time).as_secs_f32();
        self.last_frame_time = now;
        if delta > 0.0 {
            self.fps = 0.9 * self.fps + 0.1 * (1.0 / delta); // Smoothed FPS
        }

        let painter = egui_context.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("fullscreen_overlay"),
        ));

        // Read UWorld if not already cached
        if self.uworld.is_none() {
            if let Some(gworld) = self.dsapi.get_offset("OFFSET_GWORLD") {
                let full_address = self.base_address + gworld;
                tracing::debug!("Reading UWorld pointer from base+offset: 0x{:X} + 0x{:X} = 0x{:X}", self.base_address, gworld, full_address);
                match self.driver.read::<u64, u64>(full_address) {
                    Ok(uworld) => {
                        tracing::info!("UWorld address: 0x{:X}", uworld);
                        self.uworld = Some(uworld);
                    }
                    Err(e) => {
                        tracing::error!("Failed to read UWorld: {:?}", e);
                    }
                }
            }
        }

        // Read player location every frame
        if let Some(uworld) = self.uworld {
            match self.read_player_location(uworld) {
                Ok(location) => {
                    self.player_location = Some(location);
                }
                Err(_) => {
                    // Silently fail - player might not be spawned yet
                    self.player_location = None;
                }
            }
        }

        // Display info
        let mut y_offset = 16.0;
        let line_height = 20.0;
        
        let uworld_text = if let Some(uworld) = self.uworld {
            format!("UWorld: 0x{:X}", uworld)
        } else {
            "UWorld: Reading...".to_string()
        };
        painter.text(Pos2::new(16.0, y_offset), Align2::LEFT_TOP, &uworld_text, FontId::proportional(16.0), Color32::WHITE);
        y_offset += line_height;

        let location_text = if let Some(loc) = self.player_location {
            format!("Location: X={:.2}, Y={:.2}, Z={:.2}", loc.x, loc.y, loc.z)
        } else {
            "Location: Reading...".to_string()
        };
        painter.text(Pos2::new(16.0, y_offset), Align2::LEFT_TOP, &location_text, FontId::proportional(16.0), Color32::WHITE);
        y_offset += line_height;

        let fps_text = format!("FPS: {:.0}", self.fps);
        painter.text(Pos2::new(16.0, y_offset), Align2::LEFT_TOP, &fps_text, FontId::proportional(16.0), Color32::WHITE);

        /*egui::Window::new("controls").show(egui_context, |ui| {
            ui.set_width(300.0);
            self.frame += 1;
            ui.label(format!("current frame number: {}", self.frame));

            ui.label(format!(
                "pixels_per_virtual_unit: {}",
                glfw_backend.physical_pixels_per_virtual_unit
            ));
            ui.label(format!("window scale: {}", glfw_backend.scale));
            ui.label(format!("cursor pos x: {}", glfw_backend.cursor_pos[0]));
            ui.label(format!("cursor pos y: {}", glfw_backend.cursor_pos[1]));

            // quick toggles if needed while developing
            let mut borders = glfw_backend.window.is_decorated();
            if ui.checkbox(&mut borders, "window borders").changed() {
                glfw_backend.window.set_decorated(borders);
            }

            // resize helpers (disabled drag to avoid resize-jank during drag)
            let mut size = glfw_backend.window_size_logical;
            let mut changed = false;
            ui.horizontal(|ui| {
                ui.label("width: ");
                ui.add_enabled(false, DragValue::new(&mut size[0]));
                if ui.button("inc").clicked() {
                    size[0] += 10.0;
                    changed = true;
                }
                if ui.button("dec").clicked() {
                    size[0] -= 10.0;
                    changed = true;
                }
            });
            ui.horizontal(|ui| {
                ui.label("height: ");
                ui.add_enabled(false, DragValue::new(&mut size[1]));
                if ui.button("inc").clicked() {
                    size[1] += 10.0;
                    changed = true;
                }
                if ui.button("dec").clicked() {
                    size[1] -= 10.0;
                    changed = true;
                }
            });
            if changed {
                glfw_backend.set_window_size(size);
            }
        });*/

        if egui_context.wants_pointer_input() || egui_context.wants_keyboard_input() {
            glfw_backend.set_passthrough(false);
        } else {
            glfw_backend.set_passthrough(true)
        }

        egui_context.request_repaint();
    }
}

macro_rules! get_offset {
    ($dsapi:expr, $class:expr, $member:expr) => {{
        let info = $dsapi.get_member_offset($class, $member)
            .ok_or_else(|| {
                tracing::error!("Offset not found: {}::{}", $class, $member);
                crate::driver::DriverError::Status(crate::driver::DriverStatus::ReadFailed)
            })?;
        if !info.valid {
            tracing::error!("Invalid offset: {}::{}", $class, $member);
            return Err(crate::driver::DriverError::Status(crate::driver::DriverStatus::ReadFailed));
        }
        info.offset as u64
    }};
}

impl App {
    fn read_player_location(&self, uworld: u64) -> Result<FVector, crate::driver::DriverError> {
        let location = MemoryChain::new(&self.driver, &self.dsapi, uworld)?
            .follow("UWorld", "OwningGameInstance")?
            .follow("UGameInstance", "LocalPlayers")?
            .deref()? // TArray[0]
            .follow("UPlayer", "PlayerController")?
            .follow("APlayerController", "AcknowledgedPawn")?
            .follow("AActor", "RootComponent")?
            .read::<FVector>("USceneComponent", "RelativeLocation")?;

        Ok(location)
    }
}

struct MemoryChain<'a> {
    driver: &'a DriverManager,
    dsapi: &'a DSAPI,
    addr: u64,
}

impl<'a> MemoryChain<'a> {
    fn new(driver: &'a DriverManager, dsapi: &'a DSAPI, addr: u64) -> Result<Self, crate::driver::DriverError> {
        if addr == 0 {
            return Err(crate::driver::DriverError::Status(crate::driver::DriverStatus::ReadFailed));
        }
        Ok(Self { driver, dsapi, addr })
    }

    fn follow(self, class: &str, member: &str) -> Result<Self, crate::driver::DriverError> {
        let offset = get_offset!(self.dsapi, class, member);
        let next_addr: u64 = self.driver.read(self.addr + offset)?;
        //tracing::debug!("{}::{} -> 0x{:X}", class, member, next_addr);
        
        if next_addr == 0 {
            return Err(crate::driver::DriverError::Status(crate::driver::DriverStatus::ReadFailed));
        }
        
        Ok(Self { driver: self.driver, dsapi: self.dsapi, addr: next_addr })
    }

    fn deref(self) -> Result<Self, crate::driver::DriverError> {
        let next_addr: u64 = self.driver.read(self.addr)?;
        //tracing::debug!("deref -> 0x{:X}", next_addr);
        
        if next_addr == 0 {
            return Err(crate::driver::DriverError::Status(crate::driver::DriverStatus::ReadFailed));
        }
        
        Ok(Self { driver: self.driver, dsapi: self.dsapi, addr: next_addr })
    }

    fn read<T: Copy + Default>(self, class: &str, member: &str) -> Result<T, crate::driver::DriverError> {
        let offset = get_offset!(self.dsapi, class, member);
        let value: T = self.driver.read(self.addr + offset)?;
        //tracing::debug!("{}::{} -> value", class, member);
        Ok(value)
    }
}
