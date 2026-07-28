use eframe::egui::IconData;

const SIZE: u32 = 128;
const SAMPLES: u32 = 4;

pub fn app_icon() -> IconData {
    let mut rgba = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    let center = SIZE as f32 / 2.0;

    for y in 0..SIZE {
        for x in 0..SIZE {
            let mut color = [0.0_f32; 4];
            for sample_y in 0..SAMPLES {
                for sample_x in 0..SAMPLES {
                    let px = x as f32 + (sample_x as f32 + 0.5) / SAMPLES as f32;
                    let py = y as f32 + (sample_y as f32 + 0.5) / SAMPLES as f32;
                    let dx = px - center;
                    let dy = py - center;
                    let distance = dx.hypot(dy);
                    let ring = coverage((distance - 39.0).abs(), 8.5);
                    let satellite = coverage((dx - 31.0).hypot(dy + 31.0), 9.0);
                    let core = coverage(distance, 7.0);
                    let alpha = ring.max(satellite).max(core);
                    let angle = dy.atan2(dx);
                    let mix = ((angle.sin() + 1.0) * 0.5).clamp(0.0, 1.0);
                    let rgb = [
                        lerp(93.0, 139.0, mix),
                        lerp(103.0, 92.0, mix),
                        lerp(255.0, 246.0, mix),
                    ];
                    color[0] += rgb[0] * alpha;
                    color[1] += rgb[1] * alpha;
                    color[2] += rgb[2] * alpha;
                    color[3] += 255.0 * alpha;
                }
            }

            let divisor = (SAMPLES * SAMPLES) as f32;
            let alpha = color[3] / 255.0;
            if alpha > 0.0 {
                rgba.extend([
                    (color[0] / alpha).round() as u8,
                    (color[1] / alpha).round() as u8,
                    (color[2] / alpha).round() as u8,
                    (color[3] / divisor).round() as u8,
                ]);
            } else {
                rgba.extend([0, 0, 0, 0]);
            }
        }
    }

    IconData {
        rgba,
        width: SIZE,
        height: SIZE,
    }
}

fn coverage(distance: f32, radius: f32) -> f32 {
    (radius + 0.75 - distance).clamp(0.0, 1.0)
}

fn lerp(from: f32, to: f32, amount: f32) -> f32 {
    from + (to - from) * amount
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_has_the_expected_rgba_dimensions() {
        let icon = app_icon();
        assert_eq!(icon.width, SIZE);
        assert_eq!(icon.height, SIZE);
        assert_eq!(icon.rgba.len(), (SIZE * SIZE * 4) as usize);
        assert!(icon.rgba.chunks_exact(4).any(|pixel| pixel[3] > 0));
    }
}
