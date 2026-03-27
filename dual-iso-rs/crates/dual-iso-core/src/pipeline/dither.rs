use rand::Rng;

/// Apply anti-posterisation dithering.
///
/// Before rounding raw pixel values to integers, add Gaussian noise with
/// σ = 0.5 to each pixel.  This prevents banding artefacts that would
/// otherwise appear in flat gradient areas.
pub fn apply_dither(buf: &mut crate::types::RawBuffer) {
    let mut rng = rand::rng();
    for v in buf.data.iter_mut() {
        let noise = gaussian_noise(&mut rng, 0.5f32);
        let dithered = (*v as f32 + noise).round();
        *v = dithered.clamp(0.0, u16::MAX as f32) as u16;
    }
}

/// Box-Muller Gaussian sample with standard deviation `sigma`.
#[inline]
fn gaussian_noise<R: Rng>(rng: &mut R, sigma: f32) -> f32 {
    let u1: f32 = rng.random::<f32>().max(f32::EPSILON);
    let u2: f32 = rng.random::<f32>();
    let mag = sigma * (-2.0 * u1.ln()).sqrt();
    mag * (std::f32::consts::TAU * u2).cos()
}
