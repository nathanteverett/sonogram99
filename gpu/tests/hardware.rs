//! Explicit hardware tests fail if no real adapter is available; they never silently skip.
//! cargo test -p sonograph-gpu --release --test hardware -- --ignored --nocapture
use sonograph_core::{
    acceleration::{self, Accelerator, Operation, Warp},
    analysis::{Analyzer, FFT},
    canvas::{unpack, Canvas},
    glow::{Flow, Grid},
    modes,
    palette::{Palette, SKINS},
    post::{self, Look},
    preset::{Mirror, Preset},
    testsignal::TestSignal,
};
use sonograph_gpu::Gpu;

fn gpu() -> Gpu {
    let mut gpu = Gpu::new().expect("A working hardware GPU is required for this explicit test");
    eprintln!("Testing {}", gpu.name());
    gpu.set_min_pixels(0);
    gpu
}

#[test]
#[ignore = "requires a hardware GPU"]
fn shaders_preserve_warp_borders_palette_and_effects() {
    let mut gpu = gpu();
    let (w, h) = (257, 133); // Exercises partial workgroups and non-square images.
    let src: Vec<[f32; 2]> = (0..w * h)
        .map(|i| {
            let e = (i % 197) as f32 / 33.0;
            [e, e * (i % 29) as f32 / 28.0]
        })
        .collect();
    for spread in [false, true] {
        let grid = Grid::build(w, h, |x, y| [x + 0.25, y + 0.25, 0.75]);
        let mut a = vec![[0.0; 2]; w * h];
        let mut b = a.clone();
        assert!(gpu
            .run(Operation::Warp {
                src: &src,
                dst: &mut a,
                w,
                h,
                spread,
                warp: Warp::Flow {
                    from: (0.25, 0.25),
                    to: (0.0, 0.0),
                    zoom: 1.0,
                    rot: 0.0,
                    keep: 0.75,
                    flow: Flow::default()
                }
            })
            .unwrap());
        assert!(gpu
            .run(Operation::Warp {
                src: &src,
                dst: &mut b,
                w,
                h,
                spread,
                warp: Warp::Grid(&grid)
            })
            .unwrap());
        for (i, (p, q)) in a.iter().zip(&b).enumerate() {
            assert!(
                (p[0] - q[0]).abs() < 0.002 && (p[1] - q[1]).abs() < 0.002,
                "warp mismatch {i}: {p:?} vs {q:?}"
            );
            let (x, y) = (i % w, i / w);
            if x == w - 1
                || y == h - 1
                || (spread && (x == 0 || y == 0 || x >= w - 2 || y >= h - 2))
            {
                assert_eq!(*p, [0.0, 0.0]);
            }
        }
    }
    for skin in 0..SKINS.len() {
        let pal = Palette::new(skin);
        let mut pixels = vec![0; w * h];
        assert!(gpu
            .run(Operation::Resolve {
                src: &src,
                dst: &mut pixels,
                w,
                h,
                palette: &pal,
                brightness: 1.0
            })
            .unwrap());
        assert_eq!(pixels[0], sonograph_core::canvas::pack(pal.skin.screen));
        for mirror in [Mirror::None, Mirror::X, Mirror::Y, Mirror::Both] {
            let look = Look {
                echo: 0.37,
                echo_zoom: 1.03,
                gamma: 1.3,
                solarize: 0.2,
                invert: 0.17,
            };
            let mut expected = Canvas {
                w,
                h,
                px: pixels.clone(),
            };
            post::apply(&mut expected, &look, mirror, &mut Vec::new());
            let mut actual = pixels.clone();
            assert!(gpu
                .run(Operation::Post {
                    pixels: &mut actual,
                    w,
                    h,
                    look: &look,
                    mirror
                })
                .unwrap());
            compare(&expected.px, &actual, 0.02, "post effects");
        }
    }
}

fn compare(a: &[u32], b: &[u32], tolerance: f64, label: &str) {
    let mut total = 0u64;
    let mut large = 0usize;
    let mut max = 0u8;
    for (&a, &b) in a.iter().zip(b) {
        for (a, b) in unpack(a).into_iter().zip(unpack(b)) {
            let d = a.abs_diff(b);
            total += u64::from(d);
            max = max.max(d);
            if d > 8 {
                large += 1;
            }
        }
    }
    let mean = total as f64 / (a.len() * 3) as f64;
    eprintln!("{label}: mean error {mean:.4}/255, max {max}, large errors {large}");
    assert!(
        mean < tolerance && large < a.len() / 100 + 1,
        "{label}: image changed (mean {mean}, large {large})"
    );
}

#[test]
#[ignore = "requires a hardware GPU"]
fn all_visuals_match_software_after_animation_resize_and_skin_changes() {
    acceleration::software(None);
    let mut signal = TestSignal::new(48_000.0);
    let mut analyzer = Analyzer::new();
    // Build identical audio input once; GPU and CPU receive precisely the same frames.
    let mut left = vec![0.0; FFT];
    let mut right = vec![0.0; FFT];
    let mut frames = Vec::new();
    // Reach the full arrangement before comparing; the quiet intro can hide defects.
    for step in 0..384 {
        left.rotate_left(800);
        right.rotate_left(800);
        let mut i = FFT - 800;
        signal.render(800, |l, r| {
            left[i] = l;
            right[i] = r;
            i += 1;
        });
        analyzer.process(&left, &right, 48_000.0, 1.0 / 60.0);
        if step >= 360 {
            frames.push(analyzer.frame.clone());
        }
    }
    let render = || {
        let mut pictures = Vec::new();
        for (mode, mut visual) in modes::all().into_iter().enumerate() {
            visual.configure(&Preset::default());
            let mut c = Canvas::new(257, 133);
            visual.reset(&mut c);
            for (i, frame) in frames.iter().enumerate() {
                if i == 12 {
                    c.resize(311, 179);
                    visual.reset(&mut c);
                }
                let pal = Palette::new(if i < 12 { 1 } else { SKINS.len() - 1 });
                visual.draw(&mut c, frame, &pal, 1.0 / 60.0);
                if i == 11 || i == 23 {
                    pictures.push((format!("mode {} frame {i}", mode + 1), c.px.clone()));
                }
            }
        }
        pictures
    };
    let expected = render();
    acceleration::install(Box::new(gpu()));
    let actual = render();
    let status = acceleration::status();
    assert!(
        status.adapter.is_some() && status.fallback.is_none() && status.operations > 100,
        "{status:?}"
    );
    acceleration::software(None);
    for ((label, a), (_, b)) in expected.iter().zip(actual) {
        compare(a, &b, 0.8, label);
    }
}
