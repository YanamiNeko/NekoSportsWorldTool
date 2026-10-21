//! Derived statistics have one source: the final cumulative samples.
use super::model::{GenPoint, Segment, TenWindow, Track};

/// Interpolate cumulative counters at a time boundary. Rounding cumulative steps
/// (rather than each interval) preserves the exact final step count.
pub fn counters_at(locs: &[GenPoint], time: i64) -> (f64, i64) {
    let Some(first) = locs.first() else {
        return (0.0, 0);
    };
    if time <= first.totalTime {
        return (first.totalDis, first.steps);
    }
    for pair in locs.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        if b.totalTime >= time && b.totalTime > a.totalTime {
            let k = (time - a.totalTime) as f64 / (b.totalTime - a.totalTime) as f64;
            return (
                a.totalDis + (b.totalDis - a.totalDis) * k,
                (a.steps as f64 + (b.steps - a.steps) as f64 * k).round() as i64,
            );
        }
    }
    let last = locs.last().unwrap();
    (last.totalDis, last.steps)
}

pub fn rebuild(track: &mut Track) {
    let Some(last) = track.locations.last() else {
        return;
    };
    track.totalTime = last.totalTime;
    track.totalDistance = last.totalDis;
    track.totalSteps = last.steps;
    track.validDistance = last.validDis;
    track.validTime = last.validTime;
    track.speedPerTenSec.clear();
    track.stepsPerTenSec.clear();
    track.segments.clear();
    for (width, segments) in [(10, false), (60, true)] {
        let mut lo = 0;
        let mut previous = counters_at(&track.locations, 0);
        while lo < track.totalTime {
            let hi = (lo + width).min(track.totalTime);
            let current = counters_at(&track.locations, hi);
            let distance = current.0 - previous.0;
            let steps = current.1 - previous.1;
            if segments {
                track.segments.push(Segment {
                    totalTime: hi - lo,
                    distance: current.0.round() as i64 - previous.0.round() as i64,
                    startTime: lo * 1000,
                    endTime: hi * 1000,
                    avgSpeed: distance / (hi - lo) as f64,
                    avgStep: (steps as f64 * 60.0 / (hi - lo) as f64).round() as i64,
                    state: 0,
                });
            } else {
                track.speedPerTenSec.push(TenWindow {
                    time: hi - lo,
                    value: distance,
                });
                track.stepsPerTenSec.push(TenWindow {
                    time: hi - lo,
                    value: steps as f64,
                });
            }
            previous = current;
            lo = hi;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn final_samples_drive_totals_and_partial_windows() {
        for duration in [1, 9, 10, 11, 59, 60, 61, 180, 603] {
            let mut t = crate::track::generator::build(
                1200.0,
                600,
                7,
                (39.9, 116.4),
                0,
                &[(39.9, 116.4), (39.901, 116.401)],
            );
            let mut a = t.locations[0].clone();
            a.totalTime = 0;
            a.totalDis = 0.0;
            a.steps = 0;
            let mut b = a.clone();
            b.totalTime = duration;
            b.totalDis = 1000.0;
            b.steps = 1201;
            b.validTime = duration - 1;
            b.validDis = 999.0;
            t.locations = vec![a, b];
            rebuild(&mut t);
            assert_eq!(t.totalTime, duration);
            assert_eq!(t.validTime, duration - 1);
            assert_eq!(t.validDistance, 999.0);
            assert_eq!(
                t.stepsPerTenSec.iter().map(|w| w.value as i64).sum::<i64>(),
                1201
            );
            assert!((t.speedPerTenSec.iter().map(|w| w.value).sum::<f64>() - 1000.0).abs() < 1e-8);
            assert_eq!(
                t.speedPerTenSec.iter().map(|w| w.time).sum::<i64>(),
                duration
            );
            assert_eq!(t.segments[0].startTime, 0);
            assert_eq!(t.segments.iter().map(|s| s.distance).sum::<i64>(), 1000);
            assert_eq!(t.segments.last().unwrap().endTime, duration * 1000);
            for pair in t.segments.windows(2) {
                assert_eq!(pair[0].endTime, pair[1].startTime);
            }
        }
    }
}
