//! Exact bitrate selection within the timing limits reported by the adapter.

use crate::structures::{DeviceBitTiming, DeviceCapability};

struct Limits {
    tseg1_min: u32,
    tseg1_max: u32,
    tseg2_min: u32,
    tseg2_max: u32,
    sjw_max: u32,
    brp_min: u32,
    brp_max: u32,
    brp_inc: u32,
}

impl Limits {
    fn from_capability(cap: &DeviceCapability, data_phase: bool) -> Option<Self> {
        if data_phase {
            Some(Self {
                tseg1_min: cap.dtseg1_min?,
                tseg1_max: cap.dtseg1_max?,
                tseg2_min: cap.dtseg2_min?,
                tseg2_max: cap.dtseg2_max?,
                sjw_max: cap.dsjw_max?,
                brp_min: cap.dbrp_min?,
                brp_max: cap.dbrp_max?,
                brp_inc: cap.dbrp_inc?,
            })
        } else {
            Some(Self {
                tseg1_min: cap.tseg1_min,
                tseg1_max: cap.tseg1_max,
                tseg2_min: cap.tseg2_min,
                tseg2_max: cap.tseg2_max,
                sjw_max: cap.sjw_max,
                brp_min: cap.brp_min,
                brp_max: cap.brp_max,
                brp_inc: cap.brp_inc,
            })
        }
    }
}

/// Find an exact bitrate, preferring the requested sample point and then a
/// smaller prescaler. The number of bit quanta must divide clock / bitrate.
pub(crate) fn calculate_bittiming(
    cap: &DeviceCapability,
    bitrate: u32,
    data_phase: bool,
    sample_point_permille: u32,
) -> Option<DeviceBitTiming> {
    let limits = Limits::from_capability(cap, data_phase)?;
    if bitrate == 0
        || cap.fclk_can == 0
        || (cap.fclk_can / bitrate) * bitrate != cap.fclk_can
        || sample_point_permille >= 1000
        || limits.brp_inc == 0
        || limits.brp_min == 0
        || limits.sjw_max == 0
    {
        return None;
    }

    let ratio = u64::from(cap.fclk_can / bitrate);
    let mut best: Option<(u64, u64, u64, DeviceBitTiming)> = None;

    // Enumerating divisor pairs takes at most sqrt(clock / bitrate) steps.
    let mut divisor = 1u64;
    while divisor * divisor <= ratio {
        if ratio % divisor == 0 {
            for brp in [divisor, ratio / divisor] {
                if brp < u64::from(limits.brp_min)
                    || brp > u64::from(limits.brp_max)
                    || (brp - u64::from(limits.brp_min)) % u64::from(limits.brp_inc) != 0
                {
                    continue;
                }

                let tq = ratio / brp;
                if tq < 3 {
                    continue;
                }

                let low = u64::from(limits.tseg2_min)
                    .max((tq - 1).saturating_sub(u64::from(limits.tseg1_max)));
                let high = u64::from(limits.tseg2_max)
                    .min((tq - 1).saturating_sub(u64::from(limits.tseg1_min)));
                if low > high || low == 0 {
                    continue;
                }

                let ideal = tq * u64::from(1000 - sample_point_permille) / 1000;
                for tseg2 in [ideal.clamp(low, high), (ideal + 1).clamp(low, high)] {
                    let tseg1 = tq - 1 - tseg2;
                    if tseg1 == 0 || tseg1 > u64::from(u32::MAX) {
                        continue;
                    }

                    let sample_error =
                        ((tq - tseg2) * 1000).abs_diff(tq * u64::from(sample_point_permille));
                    let better = match best {
                        None => true,
                        Some((best_error, best_tq, best_brp, _)) => {
                            u128::from(sample_error) * u128::from(best_tq)
                                < u128::from(best_error) * u128::from(tq)
                                || (u128::from(sample_error) * u128::from(best_tq)
                                    == u128::from(best_error) * u128::from(tq)
                                    && brp < best_brp)
                        }
                    };
                    if better {
                        let prop_seg = if tseg1 >= 2 { 1u64 } else { 0u64 };
                        best = Some((
                            sample_error,
                            tq,
                            brp,
                            DeviceBitTiming::new(
                                prop_seg as u32,
                                (tseg1 - prop_seg) as u32,
                                tseg2 as u32,
                                1,
                                brp as u32,
                            ),
                        ));
                    }
                }
            }
        }
        divisor += 1;
    }

    best.map(|(_, _, _, timing)| timing)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capability(clock: u32) -> DeviceCapability {
        let mut cap = DeviceCapability::unpack(&[0; 40]);
        cap.fclk_can = clock;
        cap.tseg1_min = 2;
        cap.tseg1_max = 256;
        cap.tseg2_min = 1;
        cap.tseg2_max = 128;
        cap.sjw_max = 16;
        cap.brp_min = 1;
        cap.brp_max = 1024;
        cap.brp_inc = 1;
        cap.dtseg1_min = Some(2);
        cap.dtseg1_max = Some(256);
        cap.dtseg2_min = Some(1);
        cap.dtseg2_max = Some(128);
        cap.dsjw_max = Some(16);
        cap.dbrp_min = Some(1);
        cap.dbrp_max = Some(1024);
        cap.dbrp_inc = Some(1);
        cap
    }

    fn rate(clock: u32, timing: DeviceBitTiming) -> u32 {
        clock / (timing.brp * (1 + timing.prop_seg + timing.phase_seg1 + timing.phase_seg2))
    }

    #[test]
    fn selects_advertised_rates() {
        for clock in [40_000_000, 80_000_000] {
            let cap = capability(clock);
            let classic = calculate_bittiming(&cap, 800_000, false, 875).unwrap();
            let data = calculate_bittiming(&cap, 4_000_000, true, 750).unwrap();
            assert_eq!(rate(clock, classic), 800_000);
            assert_eq!(rate(clock, data), 4_000_000);
        }
    }

    #[test]
    fn respects_reported_limits() {
        let mut cap = capability(40_000_000);
        cap.brp_min = 7;
        cap.brp_max = 7;
        assert!(calculate_bittiming(&cap, 800_000, false, 875).is_none());
        cap.brp_min = 1;
        cap.brp_max = 1;
        cap.brp_inc = 0;
        assert!(calculate_bittiming(&cap, 800_000, false, 875).is_none());
    }
}
