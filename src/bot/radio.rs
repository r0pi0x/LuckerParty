//! What bots say on the team radio (`core::Radio`): "Enemy spotted" when
//! an enemy first comes into sight, "Enemy down" after a kill, "Need
//! backup" when hurt badly. Each call has its own cooldown per bot, and a
//! bot stays quiet when a teammate said the same thing a moment ago. The
//! calls are CS:S bots' radio habits as seen in play; the numbers are
//! ours (docs/tech-debt.md).

use bevy::prelude::*;

use super::Bot;
use crate::core::{Died, Health, Radio, Team};

/// Seconds before a bot repeats a call.
pub const COOLDOWN: f64 = 12.0;
/// Seconds a teammate's call keeps the rest of the team from repeating it.
pub const TEAM_QUIET: f64 = 4.0;
/// Health share at or below which a hurt bot asks for backup.
pub const BACKUP_HEALTH: f32 = 0.4;

/// A bot's radio memory: when each call may be said again, what it saw
/// last tick and its health then.
#[derive(Clone, Debug, Default)]
pub struct BotRadio {
    next: Vec<(&'static str, f64)>,
    had_target: bool,
    health: Option<f32>,
}

impl BotRadio {
    /// Whether `call` is off cooldown at `now`; if so it is used up.
    pub fn ready(&mut self, call: &'static str, now: f64) -> bool {
        match self.next.iter_mut().find(|(c, _)| *c == call) {
            Some((_, at)) if now < *at => false,
            Some((_, at)) => {
                *at = now + COOLDOWN;
                true
            }
            None => {
                self.next.push((call, now + COOLDOWN));
                true
            }
        }
    }
}

/// The call (if any) for what changed since last tick: an enemy came into
/// sight, or health fell to the backup threshold or was hit below it.
pub fn observe(radio: &mut BotRadio, has_target: bool, health: f32) -> Option<&'static str> {
    let spotted = has_target && !radio.had_target;
    let hurt = radio
        .health
        .is_some_and(|before| health < before && health <= BACKUP_HEALTH && health > 0.0);
    radio.had_target = has_target;
    radio.health = Some(health);
    if spotted {
        Some("enemyspot")
    } else if hurt {
        Some("needbackup")
    } else {
        None
    }
}

/// Recent calls per team, for `TEAM_QUIET`.
#[derive(Resource, Default)]
pub(super) struct TeamCalls(Vec<(Team, &'static str, f64)>);

#[allow(clippy::type_complexity)]
pub(super) fn speak(
    mut bots: Query<(Entity, &mut Bot, &Health, &Team)>,
    teams: Query<&Team>,
    mut died: MessageReader<Died>,
    mut radio: MessageWriter<Radio>,
    mut recent: ResMut<TeamCalls>,
    cfg: Res<super::BotConfig>,
    time: Res<Time>,
) {
    let now = time.elapsed_secs_f64();
    recent.0.retain(|(_, _, at)| now - at < TEAM_QUIET);
    let mut wanted: Vec<(Entity, &'static str)> = Vec::new();
    for d in died.read() {
        let Some(killer) = d.attacker.filter(|a| *a != d.entity) else {
            continue;
        };
        let enemies = match (teams.get(killer), teams.get(d.entity)) {
            (Ok(a), Ok(b)) => a != b,
            _ => true,
        };
        if enemies && bots.contains(killer) {
            wanted.push((killer, "enemydown"));
        }
    }
    for (e, mut bot, health, _) in &mut bots {
        let has_target = bot.target.is_some();
        if let Some(call) = observe(&mut bot.radio, has_target, health.current) {
            wanted.push((e, call));
        }
    }
    if cfg.radio == 0 {
        return;
    }
    for (e, call) in wanted {
        let Ok((_, mut bot, health, team)) = bots.get_mut(e) else {
            continue;
        };
        if health.current <= 0.0 && call != "enemydown" {
            continue;
        }
        if recent.0.iter().any(|(t, c, _)| t == team && *c == call) || !bot.radio.ready(call, now) {
            continue;
        }
        recent.0.push((*team, call, now));
        radio.write(Radio {
            sender: e,
            command: call.into(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calls_wait_out_their_cooldown() {
        let mut r = BotRadio::default();
        assert!(r.ready("enemyspot", 0.0));
        assert!(!r.ready("enemyspot", COOLDOWN - 0.1));
        assert!(r.ready("enemydown", 1.0), "each call has its own");
        assert!(r.ready("enemyspot", COOLDOWN));
    }

    #[test]
    fn spotting_and_getting_hurt() {
        let mut r = BotRadio::default();
        assert_eq!(observe(&mut r, false, 1.0), None);
        assert_eq!(observe(&mut r, true, 1.0), Some("enemyspot"));
        assert_eq!(observe(&mut r, true, 1.0), None, "same sighting");
        assert_eq!(observe(&mut r, true, 0.5), None, "still healthy");
        assert_eq!(observe(&mut r, true, 0.3), Some("needbackup"));
        assert_eq!(observe(&mut r, true, 0.3), None, "not hit again");
        assert_eq!(observe(&mut r, true, 0.0), None, "dead");
        assert_eq!(observe(&mut r, false, 1.0), None, "respawned");
    }
}
