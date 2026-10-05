//! Source WAV decoding on real files from the install (skipped without one).

use mashup::{
    games::cs_source::{self, mount, wav},
    mount::config::LocalConfig,
};

fn read(path: &str) -> Option<Vec<u8>> {
    let config = LocalConfig::load().ok()?;
    mount::open(&config.game_path(cs_source::GAME)?).ok()?.read(path).ok()
}

#[test]
fn pcm_and_adpcm() {
    let Some(step) = read("sound/player/footsteps/concrete1.wav") else {
        return;
    };
    let c = wav::decode(&step).expect("PCM step");
    assert!(c.samples.iter().any(|s| s.unsigned_abs() > 1000), "audible");
    // CS:S's own MS-ADPCM file, a loop.
    let drip = read("sound/ambient/weather/drip_loop1.wav").expect("drip loop");
    let d = wav::decode(&drip).expect("ADPCM decodes");
    let peak = d.samples.iter().map(|s| s.unsigned_abs()).max().unwrap();
    assert!(peak > 500 && peak < 32768, "peak {peak}");
}
