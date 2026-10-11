//! Native compute acceleration. The shared core remains usable without GPU dependencies.

use std::collections::HashMap;
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use sonograph_core::acceleration::{Accelerator, Operation, Warp};
use sonograph_core::{canvas::pack, preset::Mirror};

const KERNELS: [&str; 8] = [
    "warp_flow",
    "warp_grid",
    "resolve",
    "post",
    "orbs",
    "mandala",
    "kaleidoscope",
    "gravity",
];
const TIMEOUT: Duration = Duration::from_secs(3);
type Workload = (u8, usize, usize);

#[derive(Default)]
struct Timing {
    cpu: Vec<f64>,
    gpu: Vec<f64>,
}

impl Timing {
    fn use_gpu(&self) -> bool {
        if self.cpu.len() < 3 {
            return false;
        }
        if self.gpu.len() < 3 {
            return true;
        }
        let median = |values: &[f64]| {
            let mut v = values.to_vec();
            v.sort_by(f64::total_cmp);
            v[v.len() / 2]
        };
        // Require a useful margin so clock and scheduling noise do not select transfers
        // over a similarly fast CPU pass.
        median(&self.gpu) < median(&self.cpu) * 0.8
    }
}

struct Buffer {
    handle: wgpu::Buffer,
    size: u64,
    usage: wgpu::BufferUsages,
}

impl Buffer {
    fn new(device: &wgpu::Device, size: u64, usage: wgpu::BufferUsages) -> Self {
        let size = size.max(16).next_multiple_of(16);
        Self {
            handle: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Sonograph compute buffer"),
                size,
                usage,
                mapped_at_creation: false,
            }),
            size,
            usage,
        }
    }
    fn fit(&mut self, device: &wgpu::Device, size: u64) {
        if size > self.size {
            *self = Self::new(device, size, self.usage);
        }
    }
}

/// Reuses transfer and compute buffers. Small workloads stay on the CPU to avoid
/// paying GPU synchronization overhead for the tiny floating Halo or a pixelated window.
pub struct Gpu {
    name: String,
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipelines: Vec<wgpu::ComputePipeline>,
    layout: wgpu::BindGroupLayout,
    inputs: [Buffer; 3],
    output: Buffer,
    readback: Buffer,
    params: Buffer,
    failure: Arc<Mutex<Option<String>>>,
    min_pixels: usize,
    max_bytes: u64,
    timings: HashMap<Workload, Timing>,
    pending_cpu: Option<Workload>,
    pending_projection: Option<Workload>,
    calibrate: bool,
}

impl Gpu {
    pub fn new() -> Result<Self, String> {
        pollster::block_on(Self::initialize())
    }

    async fn initialize() -> Result<Self, String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::None,
                force_fallback_adapter: false,
                compatible_surface: None,
                ..Default::default()
            })
            .await
            .map_err(|e| format!("No compatible GPU: {e}"))?;
        let info = adapter.get_info();
        if info.device_type == wgpu::DeviceType::Cpu {
            return Err("Only a software graphics adapter is available".into());
        }
        let limits = adapter.limits();
        let max_bytes = limits
            .max_storage_buffer_binding_size
            .min(limits.max_buffer_size);
        let required_limits = wgpu::Limits {
            max_storage_buffer_binding_size: limits.max_storage_buffer_binding_size,
            max_buffer_size: limits.max_buffer_size,
            ..wgpu::Limits::default()
        };
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("Sonograph GPU renderer"),
                required_limits,
                ..Default::default()
            })
            .await
            .map_err(|e| format!("Could not initialize GPU: {e}"))?;
        let failure = Arc::new(Mutex::new(None));
        let error = Arc::clone(&failure);
        device.on_uncaptured_error(Arc::new(move |e: wgpu::Error| {
            *error.lock().unwrap_or_else(|e| e.into_inner()) = Some(e.to_string());
        }));
        let lost = Arc::clone(&failure);
        device.set_device_lost_callback(move |reason, message| {
            *lost.lock().unwrap_or_else(|e| e.into_inner()) =
                Some(format!("GPU lost ({reason:?}): {message}"));
        });
        let errors = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let entries: Vec<_> = (0..5)
            .map(|binding| wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: if binding == 4 {
                        wgpu::BufferBindingType::Uniform
                    } else {
                        wgpu::BufferBindingType::Storage {
                            read_only: binding != 3,
                        }
                    },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            })
            .collect();
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Sonograph compute layout"),
            entries: &entries,
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Sonograph pipelines"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Sonograph pixel kernels"),
            source: wgpu::ShaderSource::Wgsl(include_str!("pixels.wgsl").into()),
        });
        let pipelines = KERNELS
            .iter()
            .map(|entry| {
                device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(entry),
                    layout: Some(&pipeline_layout),
                    module: &shader,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    cache: None,
                })
            })
            .collect();
        if let Some(e) = errors.pop().await {
            return Err(format!("GPU shaders unavailable: {e}"));
        }
        let inputs = std::array::from_fn(|_| {
            Buffer::new(
                &device,
                16,
                wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            )
        });
        let output = Buffer::new(
            &device,
            16,
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        );
        let readback = Buffer::new(
            &device,
            16,
            wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        );
        let params = Buffer::new(
            &device,
            256,
            wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        );
        Ok(Self {
            name: format!("{} ({:?})", info.name, info.backend),
            device,
            queue,
            pipelines,
            layout,
            inputs,
            output,
            readback,
            params,
            failure,
            min_pixels: 256 * 256,
            max_bytes,
            timings: HashMap::new(),
            pending_cpu: None,
            pending_projection: None,
            calibrate: true,
        })
    }

    /// Force the GPU above this size, bypassing calibration for explicit hardware tests.
    pub fn set_min_pixels(&mut self, pixels: usize) {
        self.min_pixels = pixels;
        self.calibrate = false;
    }

    fn dispatch(
        &mut self,
        kernel: usize,
        params: &[f32; 64],
        inputs: [&[u8]; 3],
        output: &mut [u8],
    ) -> Result<bool, String> {
        if let Some(e) = self
            .failure
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
        {
            return Err(e);
        }
        let (w, h) = (params[0] as u32, params[1] as u32);
        if w < 2 || h < 2 || (w as usize * h as usize) < self.min_pixels {
            return Ok(false);
        }
        let bytes = output.len() as u64;
        if std::iter::once(bytes)
            .chain(inputs.iter().map(|p| p.len() as u64))
            .any(|n| n.next_multiple_of(16) > self.max_bytes)
        {
            return Err("Picture exceeds this GPU's buffer limit".into());
        }
        if w.div_ceil(8) > self.device.limits().max_compute_workgroups_per_dimension
            || h.div_ceil(8) > self.device.limits().max_compute_workgroups_per_dimension
        {
            return Err("Picture exceeds this GPU's dispatch limit".into());
        }
        for (buffer, data) in self.inputs.iter_mut().zip(inputs) {
            buffer.fit(&self.device, data.len() as u64);
            if !data.is_empty() {
                self.queue.write_buffer(&buffer.handle, 0, data);
            }
        }
        self.output.fit(&self.device, bytes);
        self.readback.fit(&self.device, bytes);
        self.queue
            .write_buffer(&self.params.handle, 0, bytemuck::cast_slice(params));
        let buffers = [
            &self.inputs[0],
            &self.inputs[1],
            &self.inputs[2],
            &self.output,
            &self.params,
        ];
        let entries: Vec<_> = buffers
            .iter()
            .enumerate()
            .map(|(i, b)| wgpu::BindGroupEntry {
                binding: i as u32,
                resource: b.handle.as_entire_binding(),
            })
            .collect();
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Sonograph frame"),
            layout: &self.layout,
            entries: &entries,
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor::default());
            pass.set_pipeline(&self.pipelines[kernel]);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
        }
        encoder.copy_buffer_to_buffer(&self.output.handle, 0, &self.readback.handle, 0, bytes);
        let submission = self.queue.submit([encoder.finish()]);
        let (send, receive) = mpsc::sync_channel(1);
        self.readback
            .handle
            .slice(..bytes)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = send.send(result);
            });
        let polled = self.device.poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: Some(TIMEOUT),
        });
        if let Err(e) = polled {
            self.readback.handle.unmap();
            return Err(format!("GPU wait failed: {e}"));
        }
        let mapped = receive
            .recv_timeout(TIMEOUT)
            .map_err(|e| format!("GPU readback timed out: {e}"))?
            .map_err(|e| format!("GPU readback failed: {e}"));
        if let Err(e) = mapped {
            self.readback.handle.unmap();
            return Err(e);
        }
        if let Some(e) = self
            .failure
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
        {
            self.readback.handle.unmap();
            return Err(e);
        }
        // Commit only after success: on any failure the caller can retry the same input.
        {
            let view = self
                .readback
                .handle
                .slice(..bytes)
                .get_mapped_range()
                .map_err(|e| format!("GPU mapping failed: {e}"))?;
            output.copy_from_slice(&view);
        }
        self.readback.handle.unmap();
        Ok(true)
    }
}

impl Accelerator for Gpu {
    fn name(&self) -> &str {
        &self.name
    }

    fn run(&mut self, operation: Operation<'_>) -> Result<bool, String> {
        if let Some(error) = self
            .failure
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
        {
            return Err(error);
        }
        let key = match &operation {
            Operation::Warp {
                w, h, warp, spread, ..
            } => (
                match (warp, spread) {
                    (Warp::Flow { .. }, false) => 0,
                    (Warp::Grid(_), false) => 1,
                    (Warp::Flow { .. }, true) => 8,
                    (Warp::Grid(_), true) => 9,
                },
                *w,
                *h,
            ),
            Operation::Resolve { w, h, .. } => (2, *w, *h),
            Operation::Post { w, h, .. } => (3, *w, *h),
            Operation::Orbs { w, h, .. } => (4, *w, *h),
            Operation::Mandala { w, h, .. } => (5, *w, *h),
            Operation::Kaleidoscope { w, h, .. } => (6, *w, *h),
            Operation::Gravity { w, h, .. } => (7, *w, *h),
        };
        self.pending_cpu = None;
        // Silence uses the Orbs software fast path; it must not decide the backend
        // for the much more expensive membrane once music starts.
        if self.calibrate && matches!(&operation, Operation::Orbs { energy, .. } if *energy < 1e-5)
        {
            return Ok(false);
        }
        if matches!(key.0, 5 | 6) {
            self.pending_projection = None;
        }
        if key.1 < 2 || key.2 < 2 || key.1.saturating_mul(key.2) < self.min_pixels {
            return Ok(false);
        }
        // These stand-alone streaming passes do less work than their round-trip transfer.
        // Color resolution remains on the GPU when fused with mirror projection.
        if self.calibrate && matches!(key.0, 0..=2) {
            return Ok(false);
        }
        if self.calibrate {
            // Bound history while resizing; each resolution gets a fresh comparison.
            if !self.timings.contains_key(&key) && self.timings.len() >= 64 {
                self.timings.clear();
            }
            if !self.timings.entry(key).or_default().use_gpu() {
                if matches!(key.0, 5 | 6) {
                    self.pending_projection = Some(key);
                } else {
                    self.pending_cpu = Some(key);
                }
                return Ok(false);
            }
        }
        let started = Instant::now();
        let result = self.execute(operation);
        if self.calibrate && matches!(result, Ok(true)) {
            let timing = self.timings.entry(key).or_default();
            if timing.gpu.len() < 3 {
                timing.gpu.push(started.elapsed().as_secs_f64());
            }
        }
        result
    }

    fn software_time(&mut self, elapsed: Duration) {
        if let Some(key) = self.pending_cpu.take() {
            let timing = self.timings.entry(key).or_default();
            if timing.cpu.len() < 3 {
                timing.cpu.push(elapsed.as_secs_f64());
            }
        }
    }
    fn projection_software_time(&mut self, elapsed: Duration) {
        if let Some(key) = self.pending_projection.take() {
            let timing = self.timings.entry(key).or_default();
            if timing.cpu.len() < 3 {
                timing.cpu.push(elapsed.as_secs_f64());
            }
        }
    }
}

impl Gpu {
    fn execute(&mut self, operation: Operation<'_>) -> Result<bool, String> {
        let mut p = [0.0f32; 64];
        match operation {
            Operation::Warp {
                src,
                dst,
                w,
                h,
                warp,
                spread,
            } => {
                p[0..4].copy_from_slice(&[w as f32, h as f32, spread as u32 as f32, 0.0]);
                let (kernel, grid) = match warp {
                    Warp::Flow {
                        from,
                        to,
                        zoom,
                        rot,
                        keep,
                        flow,
                    } => {
                        let (sn, cs) = rot.sin_cos();
                        p[4..16].copy_from_slice(&[
                            from.0,
                            from.1,
                            to.0,
                            to.1,
                            sn,
                            cs,
                            1.0 / zoom,
                            keep,
                            flow.time,
                            flow.amount,
                            flow.dt,
                            w.min(h) as f32,
                        ]);
                        (0, &[][..])
                    }
                    Warp::Grid(grid) => {
                        p[4] = grid.cols as f32;
                        p[5] = grid.rows as f32;
                        (1, bytemuck::cast_slice(&grid.pts))
                    }
                };
                self.dispatch(
                    kernel,
                    &p,
                    [bytemuck::cast_slice(src), grid, &[]],
                    bytemuck::cast_slice_mut(dst),
                )
            }
            Operation::Resolve {
                src,
                dst,
                w,
                h,
                palette,
                brightness,
            } => {
                p[0..3].copy_from_slice(&[w as f32, h as f32, brightness]);
                p[4..7].copy_from_slice(&palette.skin.screen.map(f32::from));
                p[8..11].copy_from_slice(&palette.skin.peak.map(f32::from));
                let colors = palette.glow.map(pack);
                self.dispatch(
                    2,
                    &p,
                    [
                        bytemuck::cast_slice(src),
                        bytemuck::cast_slice(&colors),
                        &[],
                    ],
                    bytemuck::cast_slice_mut(dst),
                )
            }
            Operation::Post {
                pixels,
                w,
                h,
                look,
                mirror,
            } => {
                p[0..4].copy_from_slice(&[w as f32, h as f32, look.echo, look.echo_zoom]);
                p[4] = if matches!(mirror, Mirror::X | Mirror::Both) {
                    -1.0
                } else {
                    1.0
                };
                p[5] = if matches!(mirror, Mirror::Y | Mirror::Both) {
                    -1.0
                } else {
                    1.0
                };
                let tones =
                    (look.gamma - 1.0).abs() > 1e-3 || look.solarize > 1e-3 || look.invert > 1e-3;
                let table: Vec<u32> = (0..256)
                    .map(|v| {
                        if !tones {
                            return v;
                        }
                        let mut c = (v as f32 / 255.0).powf(1.0 / look.gamma);
                        c += (4.0 * c * (1.0 - c) - c) * look.solarize;
                        c += (1.0 - 2.0 * c) * look.invert;
                        (c.clamp(0.0, 1.0) * 255.0).round() as u32
                    })
                    .collect();
                // Keep the input separate from the output until GPU readback succeeds.
                let source = pixels.to_vec();
                self.dispatch(
                    3,
                    &p,
                    [
                        bytemuck::cast_slice(&source),
                        bytemuck::cast_slice(&table),
                        &[],
                    ],
                    bytemuck::cast_slice_mut(pixels),
                )
            }
            Operation::Orbs {
                dst,
                w,
                h,
                xs,
                ys,
                lobes,
                hand,
                hue,
                energy,
                detail,
            } => {
                p[0..4].copy_from_slice(&[w as f32, h as f32, energy, detail]);
                p[4..8].copy_from_slice(&[hand[0], hand[1], hand[2], hue]);
                p[8] = lobes.len() as f32;
                let coordinates: Vec<[f32; 2]> =
                    xs.iter().chain(ys).map(|&(a, b)| [a, b]).collect();
                let lobes: Vec<[f32; 4]> = lobes.iter().map(|&(x, y, r, s)| [x, y, r, s]).collect();
                self.dispatch(
                    4,
                    &p,
                    [
                        bytemuck::cast_slice(&coordinates),
                        bytemuck::cast_slice(&lobes),
                        &[],
                    ],
                    bytemuck::cast_slice_mut(dst),
                )
            }
            Operation::Mandala {
                src,
                side,
                dst,
                palette,
                w,
                h,
                mid,
                window,
                scale,
                turn,
                folds,
            } => {
                let (sn, cs) = turn.sin_cos();
                p[0..4].copy_from_slice(&[
                    w as f32,
                    h as f32,
                    side as f32,
                    std::f32::consts::PI / folds as f32,
                ]);
                p[4..10].copy_from_slice(&[mid, window, scale, sn, cs, 0.0]);
                p[40..43].copy_from_slice(&palette.skin.screen.map(f32::from));
                p[44..47].copy_from_slice(&palette.skin.peak.map(f32::from));
                let colors = palette.glow.map(pack);
                self.dispatch(
                    5,
                    &p,
                    [
                        bytemuck::cast_slice(src),
                        &[],
                        bytemuck::cast_slice(&colors),
                    ],
                    bytemuck::cast_slice_mut(dst),
                )
            }
            Operation::Kaleidoscope {
                src,
                side,
                dst,
                palette,
                w,
                h,
                center,
                shift_x,
                shift_y,
                current,
                previous,
            } => {
                p[0..4].copy_from_slice(&[
                    w as f32,
                    h as f32,
                    side as f32,
                    previous.as_ref().map_or(1.0, |(_, t)| *t),
                ]);
                p[4..7].copy_from_slice(&[center.0, center.1, previous.is_some() as u32 as f32]);
                p[8..24].copy_from_slice(&current);
                if let Some((lens, _)) = previous {
                    p[24..40].copy_from_slice(&lens);
                }
                let shifts: Vec<f32> = shift_x.iter().chain(shift_y).copied().collect();
                p[40..43].copy_from_slice(&palette.skin.screen.map(f32::from));
                p[44..47].copy_from_slice(&palette.skin.peak.map(f32::from));
                let colors = palette.glow.map(pack);
                self.dispatch(
                    6,
                    &p,
                    [
                        bytemuck::cast_slice(src),
                        bytemuck::cast_slice(&shifts),
                        bytemuck::cast_slice(&colors),
                    ],
                    bytemuck::cast_slice_mut(dst),
                )
            }
            Operation::Gravity {
                dst,
                w,
                h,
                rings,
                lenses,
                now,
                stretch,
                moved,
                inner,
                fade,
                far,
                haze,
                haze_u,
                haze_r,
            } => {
                p[0..4].copy_from_slice(&[
                    w as f32,
                    h as f32,
                    rings.len() as f32,
                    lenses.len() as f32,
                ]);
                p[4..8].copy_from_slice(&[now.0, now.1, stretch, moved as u32 as f32]);
                p[8..14].copy_from_slice(&[inner, fade, far, haze, haze_u, haze_r]);
                let lenses: Vec<[f32; 3]> = lenses.iter().map(|&(x, y, r)| [x, y, r]).collect();
                self.dispatch(
                    7,
                    &p,
                    [
                        bytemuck::cast_slice(rings),
                        bytemuck::cast_slice(&lenses),
                        &[],
                    ],
                    bytemuck::cast_slice_mut(dst),
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Timing;
    #[test]
    fn automatic_requires_samples_and_a_clear_speed_improvement() {
        let mut timing = Timing::default();
        assert!(!timing.use_gpu());
        timing.cpu = vec![10.0, 9.0, 11.0];
        assert!(
            timing.use_gpu(),
            "GPU must be measured before selecting a path"
        );
        timing.gpu = vec![9.0, 10.0, 9.5];
        assert!(
            !timing.use_gpu(),
            "A marginal improvement is not worth transfers"
        );
        timing.gpu = vec![2.0, 2.1, 50.0];
        assert!(
            timing.use_gpu(),
            "One startup outlier must not dominate the median"
        );
    }
}
