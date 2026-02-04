//! Utilities for getting system info

use std::error::Error;
use std::sync::LazyLock;

use either::Either::{self, Left, Right};
use nvml_wrapper::enum_wrappers::device::TemperatureSensor;
use nvml_wrapper::{Device, Nvml};
use sysinfo::{Component, Components};
use zoom_sync_core::Board;

#[derive(Clone, Debug, bpaf::Bpaf)]
pub enum CpuMode {
    Label(
        /// Sensor label to search for
        #[bpaf(long("cpu"), argument("LABEL"), fallback("Package".into()), display_fallback)]
        String,
    ),
    Manual(
        /// Manually set CPU temperature
        #[bpaf(short('c'), long("cpu-temp"), argument("TEMP"))]
        u8,
    ),
}

impl CpuMode {
    pub fn either(&self) -> Either<CpuTemp, u8> {
        match self {
            CpuMode::Label(label) => Either::Left(CpuTemp::new(label)),
            CpuMode::Manual(v) => Either::Right(*v),
        }
    }
}

#[derive(Clone, Debug, bpaf::Bpaf)]
pub enum GpuMode {
    Id(
        /// GPU device id to fetch temperature data for (nvidia only)
        #[bpaf(long("gpu"), argument::<u32>("ID"), fallback(0), display_fallback)]
        u32,
    ),
    Manual(
        /// Manually set GPU stats, temperature. Disables automatic fetching.
        #[bpaf(long("gpu-temp"), argument("TEMP"), fallback(0))]
        u32,
        /// Manually set GPU stats, fanspeed. Disables automatic fetching.
        #[bpaf(long("gpu-fanspeed"), argument("FAN-SPEED"), fallback(0))]
        u32
    ),
}

impl GpuMode {
    pub fn either(&self) -> Either<GpuStats, (u32, u32)> {
        match self {
            GpuMode::Id(i) => Either::Left(GpuStats::new(*i)),
            GpuMode::Manual(temp, fanspeed) => Either::Right((*temp, *fanspeed)),
        }
    }
}

/// Helper struct to track gpu temperature
pub struct GpuStats {
    maybe_device: Option<Device<'static>>,
}

impl GpuStats {
    /// Construct a new gpu monitor, optionally selecting by device index.
    /// Handles both gpu temp and gpu fan speeds.
    pub fn new(index: u32) -> Self {
        static NVML: LazyLock<Option<Nvml>> = LazyLock::new(|| {
            let nvml = Nvml::init().ok();
            if nvml.is_none() {
                eprintln!("warning: nvml not found (nvidia gpu temp unavailable)");
            }
            nvml
        });

        let maybe_device = NVML.as_ref().and_then(|nvml| {
            let device = nvml.device_by_index(index).ok();
            if device.is_none() {
                eprintln!("warning: gpu device {index} not found")
            }
            device
        });

        Self { maybe_device }
    }

    // Refresh and poll the current temperature
    pub fn get_temp(&self, farenheit: bool) -> Option<u32> {
        self.maybe_device
            .as_ref()
            .and_then(|d| d.temperature(TemperatureSensor::Gpu).ok())
            .map(|v| {
                if farenheit {
                    (v as f64 * 9. / 5. + 32.) as u32
                } else {
                    v as u32
                }
            })
    }

    // Refresh and poll the current GPU fanspeed
    pub fn get_fanspeed(&self) -> Option<u32> {
        self.maybe_device
            .as_ref()
            .and_then(|d| d.fan_speed_rpm(0).ok())
    }
}

pub struct CpuTemp {
    maybe_cpu: Option<Component>,
}

impl CpuTemp {
    // Create a new cpu temp monitor, optionally selecting the component by a label search string
    pub fn new(search_label: &str) -> Self {
        let comps: Vec<_> = Components::new_with_refreshed_list().into();

        // Try to find the specified sensor, or fall back to common alternatives
        let fallbacks = ["Tctl", "Package", "CPU"];
        let mut matched_fallback = None;

        let maybe_cpu = comps
            .into_iter()
            .find(|v| {
                if v.label().contains(search_label) {
                    return true;
                }
                // Check fallbacks while iterating
                if matched_fallback.is_none() {
                    for (i, fb) in fallbacks.iter().enumerate() {
                        if v.label().contains(fb) {
                            matched_fallback = Some(i);
                            break;
                        }
                    }
                }
                false
            })
            .or_else(|| {
                // Didn't find exact match, try fallbacks
                if let Some(fb_idx) = matched_fallback {
                    let comps: Vec<_> = Components::new_with_refreshed_list().into();
                    let fb = fallbacks[fb_idx];
                    return comps.into_iter().find(|v| v.label().contains(fb));
                }
                None
            });

        if maybe_cpu.is_none() {
            let comps: Vec<_> = Components::new_with_refreshed_list().into();
            eprintln!("warning: no cpu temp sensor found");
            if !comps.is_empty() {
                eprintln!("  available sensors:");
                for c in &comps {
                    eprintln!("    - {}", c.label());
                }
            }
        }
        Self { maybe_cpu }
    }

    // Refresh and poll the current temperature
    pub fn get_temp(&mut self, farenheit: bool) -> Option<u8> {
        self.maybe_cpu.as_mut().map(|cpu| {
            cpu.refresh();
            match cpu.temperature() {
                Some(mut temp) => {
                    if farenheit {
                        temp = temp * 9. / 5. + 32.;
                    }
                    temp as u8
                },
                None => 0,
            }
        })
    }
}

pub fn apply_system(
    board: &mut dyn Board,
    farenheit: bool,
    cpu: &mut Either<CpuTemp, u8>,
    gpu: &Either<GpuStats, (u32, u32)>,
    download: Option<f32>,
) -> Result<(), Box<dyn Error>> {
    let system_info = board
        .as_system_info()
        .ok_or("board does not support system info")?;

    let mut cpu_temp = cpu
        .as_mut()
        .map_left(|c| c.get_temp(farenheit).unwrap_or_default())
        .map_right(|v| *v)
        .into_inner();
    if cpu_temp >= 100 {
        eprintln!("warning: actual cpu temperature at {cpu_temp}, clamping to 99");
        cpu_temp = 99;
    }

    let gpu_temp_pull = gpu
        .as_ref()
        .map_left(|g| g.get_temp(farenheit).unwrap_or_default())
        .map_right(|v| *v);

    let gpu_fan_speed_pull = gpu
        .as_ref()
        .map_left(|g| g.get_fanspeed().unwrap_or_default())
        .map_right(|v| *v);

    let mut gpu_temp: u32;

    match gpu_temp_pull {
        Left(temp) => {
            gpu_temp = temp;

        }
        Right((temp, _)) => {
            gpu_temp = temp;
        }
    }

    if gpu_temp >= 100 {
        eprintln!("warning: actual gpu temperature at {gpu_temp}. clamping to 99");
        gpu_temp = 99;
    }

    let mut gpu_fan_speed: u32;

    match gpu_fan_speed_pull {
        Left(fanspeed) => {
            gpu_fan_speed = fanspeed;

        }
        Right((_, fanspeed)) => {
            gpu_fan_speed = fanspeed;
        }
    }

    let download = download.unwrap_or_default();

    system_info
        .set_system_info(cpu_temp, gpu_temp, download, gpu_fan_speed)
        .map_err(|e| format!("failed to set system info: {e}"))?;
    println!(
        "updated system info {{ cpu_temp: {cpu_temp}, gpu_temp: {gpu_temp}, download: {download}, gpu_fan_speed: {gpu_fan_speed} }}"
    );

    Ok(())
}
