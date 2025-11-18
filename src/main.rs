// #![windows_subsystem = "windows"]  // Disabled so we can see console output

mod app;
mod driver;
use app::App;
use dumpspace_api::DSAPI;

fn main() {
    use tracing_subscriber::{fmt, prelude::*, EnvFilter};
    // Initialize logging FIRST so we can see all operations
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(
            EnvFilter::try_from_default_env()
                .unwrap_or(EnvFilter::new("trace,wgpu=warn,naga=warn")),
        )
        .init();

    tracing::info!("Starting f84ext_rs");
    tracing::info!("Opening driver: colonelLink");
    
    let mut dm = match driver::DriverManager::new("colonelLink") {
        Ok(dm) => {
            tracing::info!("Driver opened successfully");
            dm
        }
        Err(e) => {
            tracing::error!("Failed to open driver: {:?}", e);
            panic!("Failed to open driver: {:?}", e);
        }
    };

    tracing::info!("Attempting to attach to process: SolarlandClient-Win64-Shipping.exe");
    let base = dm.attach_to_process("SolarlandClient-Win64-Shipping.exe", false);

    let base_addr = match base {
        Ok(Some(base_addr)) => {
            tracing::info!("Attached to process. Base address: 0x{:X}", base_addr);
            println!("Attached to process. Base address: 0x{:X}", base_addr);
            base_addr
        }
        Ok(None) => {
            tracing::warn!("Attached to process but no base address returned");
            println!("Attached to process but no base address returned");
            panic!("No base address returned");
        }
        Err(e) => {
            tracing::error!("Failed to attach to process: {:?}", e);
            panic!("Failed to attach to process: {:?}", e);
        }
    };

    let base = base.unwrap().unwrap() as u64;

    let mut dsapi = DSAPI::new("0943efc5", None);
    dsapi.download_content().unwrap();

    tracing::info!("Starting overlay");
    let app = App {
        frame: 0,
        fullscreen_applied: false,
        driver: dm,
        dsapi,
        base_address: base_addr as u64,
        uworld: None,
        player_location: None,
        last_frame_time: std::time::Instant::now(),
        fps: 0.0,
    };
    egui_overlay::start(app);
}
// Rendering is implemented in `app.rs` and wired via EguiOverlay