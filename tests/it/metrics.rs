//! Performance metrics (`mashup::metrics`, `client::frame_metrics`):
//! ticks and frames are measured with thread CPU time (and hardware
//! counters where the kernel allows them), schedules are named in order,
//! and a frame far slower than the median is flagged as a hitch.

use std::time::Duration;

use bevy::prelude::*;
use mashup::{
    client::frame_metrics::{FrameMetrics, FrameMetricsPlugin, HitchRatio},
    console::ConsolePlugin,
    greybox::GreyboxMapPlugin,
    harness::Sim,
    metrics::{TickMetrics, hw_status},
};

#[test]
fn every_tick_is_measured() {
    let mut sim = Sim::new(GreyboxMapPlugin);
    let before = sim.app.world().resource::<TickMetrics>().ticks;
    sim.ticks(50);
    let m = sim.app.world().resource::<TickMetrics>();
    assert_eq!(m.ticks - before, 50);
    let cpu = m.work.cpu_ms.ring.percentiles().expect("cpu times");
    assert!(cpu.total > 0.0, "{cpu:?}");
    match m.work.instructions.ring.percentiles() {
        // A greybox tick runs a few hundred systems: well over 10k
        // instructions.
        Some(i) => assert!(i.p50 > 1e4, "{i:?}"),
        None => eprintln!("hardware counters: {}", hw_status()),
    }
}

#[derive(Resource, Default)]
struct UpdateCount(u32);

/// Frame 100 sleeps 40 ms (the others take microseconds).
fn slow_frame(mut n: ResMut<UpdateCount>) {
    n.0 += 1;
    if n.0 == 100 {
        std::thread::sleep(Duration::from_millis(40));
    }
}

#[test]
fn frames_are_measured_and_a_slow_one_is_a_hitch() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, ConsolePlugin, FrameMetricsPlugin))
        .init_resource::<UpdateCount>()
        .add_systems(Update, slow_frame);
    app.finish();
    app.cleanup();
    for _ in 0..120 {
        app.update();
    }
    let world = app.world();
    assert_eq!(world.resource::<HitchRatio>().0, 2.0);
    let m = world.resource::<FrameMetrics>();
    let names = m.schedule_names();
    let first = names.iter().position(|n| n == "First").expect("First measured");
    let update = names.iter().position(|n| n == "Update").expect("Update measured");
    assert!(
        first < update && names.last().map(String::as_str) == Some("Last"),
        "{names:?}"
    );
    // The first frame has no start before it.
    assert_eq!(m.frames, 119);
    assert_eq!(m.main.wall_ms.ring.len(), 120);
    assert!(m.frame_ms.ring.percentiles().unwrap().max >= 40.0);
    assert!(m.hitch_frames.contains(&100), "{:?}", m.hitch_frames);
}
