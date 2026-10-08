//! Bots answering teammates' radio commands (`bot::radio::obey`): a
//! person's "Follow me" makes the nearest bot answer and follow them,
//! "Hold this position" keeps bots near the caller's spot, all the same
//! whatever the entity ids (`MASHUP_TEST_PAD`); on de_dust2, skipped
//! without a CS:S install. Bots' own calls (`bot::radio::speak`): one
//! leader call as a round opens, "Cover me" from a planting bot, the same
//! calls whatever the entity ids, and no more than one call per team per
//! `TEAM_GAP` over a whole round. And what the player hears: `ignorerad`
//! hides teammates' calls, and players joining a team get the game's chat
//! line.

use avian3d::prelude::Position;
use bevy::{
    ecs::message::{MessageCursor, Messages},
    prelude::*,
};
use mashup::{
    bot::{
        Activity, Bot, BotConfig, Tactics,
        radio::{CALLS, HOLD_NEAR, Order, TEAM_GAP},
    },
    client::{
        chat::{ChatLine, GameMessagesPlugin},
        radio::RadioHearPlugin,
    },
    console::Console,
    core::{LocalPlayer, Radio, SpawnPoint, Team},
    games::{
        self,
        cs_source::{
            self, TICK_INTERVAL,
            movement::{self, SourceMovementPlugin, to_engine},
            objectives::C4,
            weapons::CsWeaponsPlugin,
        },
    },
    harness::Sim,
    map::{
        MapPlugin,
        nav::NavMesh,
        radio::{RadioCommand, RadioCommands, SayFormats},
    },
    mount::config::LocalConfig,
    movement::placeholder,
    rules::rounds::{Phase, RoundState},
    weapon::give,
};

fn installed() -> bool {
    let ok = LocalConfig::load()
        .ok()
        .and_then(|c| c.game_path(cs_source::GAME))
        .is_some_and(|p| p.join("cstrike").is_dir());
    if !ok {
        eprintln!("skipping: no CS:S install configured");
    }
    ok
}

/// de_dust2 with a person at the first counter-terrorist spawn and three
/// counter-terrorist bots, padded by `pad` more entities.
fn dust2(pad: usize) -> (Sim, Entity, Vec<Entity>) {
    let map = games::load_map("cs_source:de_dust2").expect("load de_dust2");
    let mut sim = Sim::new((MapPlugin::new(map), SourceMovementPlugin, CsWeaponsPlugin));
    sim.pad(pad);
    sim.set_tick_interval(TICK_INTERVAL);
    sim.app
        .insert_resource(mashup::slots::Loadout { movement: movement::ID });
    sim.app.world_mut().resource_mut::<BotConfig>().grenades = 0;
    let spawn = sim
        .app
        .world_mut()
        .query::<(&Transform, &SpawnPoint)>()
        .iter(sim.app.world())
        .find(|(_, s)| s.team == Some(Team(2)))
        .map(|(t, _)| t.translation)
        .expect("a CT spawn");
    let me = sim.spawn_character(spawn, movement::ID);
    sim.app
        .world_mut()
        .entity_mut(me)
        .insert((Team(2), Name::new("Player")));
    let bots: Vec<Entity> = (0..3)
        .map(|_| mashup::bot::add_bot(sim.app.world_mut(), Team(2)).expect("bot"))
        .collect();
    sim.ticks(2);
    (sim, me, bots)
}

fn feet(sim: &Sim, e: Entity) -> Vec3 {
    sim.position(e) - Vec3::Y * 0.9
}

fn number(sim: &Sim, e: Entity) -> u32 {
    sim.app.world().get::<Bot>(e).unwrap().number()
}

fn call(sim: &mut Sim, from: Entity, command: &str) {
    sim.app.world_mut().write_message(Radio {
        sender: from,
        command: command.into(),
    });
}

/// Step `secs`, collecting radio calls as (tick, sender, command).
fn run(sim: &mut Sim, secs: f64, cursor: &mut MessageCursor<Radio>) -> Vec<(u64, Entity, String)> {
    let mut out = Vec::new();
    for _ in 0..(secs / TICK_INTERVAL).round() as u64 {
        sim.ticks(1);
        let tick = sim.tick();
        let messages = sim.app.world().resource::<Messages<Radio>>();
        out.extend(cursor.read(messages).map(|r| (tick, r.sender, r.command.clone())));
    }
    out
}

/// A nav area's middle 12-20 m from `from` at about its height, for the
/// person to walk to (the first such area in the mesh's order).
fn somewhere_near(sim: &Sim, from: Vec3) -> Vec3 {
    let nav = sim.app.world().resource::<NavMesh>();
    let area = nav
        .areas
        .iter()
        .find(|a| {
            let d = (a.center - from).xz().length();
            (12.0..20.0).contains(&d) && (a.center.y - from.y).abs() < 1.0 && (a.max - a.min).min_element() > 1.5
        })
        .expect("an area 12-20 m away");
    area.center
}

/// "Follow me": what was said (bot number, tick, call) and which bot
/// follows; then the person walks away and the follower keeps up.
fn follow_me(pad: usize) -> (Vec<(u32, u64, String)>, u32) {
    let (mut sim, me, bots) = dust2(pad);
    let mut cursor = sim.app.world().resource::<Messages<Radio>>().get_cursor_current();
    call(&mut sim, me, "followme");
    let heard = run(&mut sim, 3.0, &mut cursor);
    let said: Vec<(u32, u64, String)> = heard
        .iter()
        .filter(|h| h.1 != me)
        .map(|h| (number(&sim, h.1), h.0, h.2.clone()))
        .collect();
    assert_eq!(
        said.iter().filter(|s| s.2 == "roger").count(),
        1,
        "one bot acknowledges: {said:?}"
    );
    let followers: Vec<Entity> = bots
        .iter()
        .copied()
        .filter(|&b| {
            sim.app
                .world()
                .get::<Bot>(b)
                .and_then(|b| b.order())
                .is_some_and(|o| matches!(o.order, Order::Follow { leader, .. } if leader == me))
        })
        .collect();
    assert_eq!(followers.len(), 1, "one bot follows");
    let follower = followers[0];
    assert_eq!(
        heard.iter().find(|h| h.2 == "roger").map(|h| h.1),
        Some(follower),
        "the follower is the one answering"
    );
    // The person walks off (teleported): the follower comes after them.
    let to = somewhere_near(&sim, feet(&sim, me));
    sim.app.world_mut().get_mut::<Transform>(me).unwrap().translation = to + Vec3::Y * 1.0;
    run(&mut sim, 12.0, &mut cursor);
    let bot = sim.app.world().get::<Bot>(follower).unwrap();
    assert_eq!(bot.activity(), Activity::Obeying);
    let d = (feet(&sim, follower) - feet(&sim, me)).xz().length();
    assert!(d < 6.0, "the follower is {d:.1} m from the person");
    (said, number(&sim, follower))
}

#[test]
fn follow_me_makes_one_bot_answer_and_follow() {
    if !installed() {
        return;
    }
    follow_me(0);
}

#[test]
fn answers_and_followers_dont_depend_on_entity_ids() {
    if !installed() {
        return;
    }
    let base = follow_me(0);
    for pad in [7, 40] {
        assert_eq!(follow_me(pad), base, "padded by {pad}");
    }
}

#[test]
fn hold_this_position_keeps_bots_near_the_caller() {
    if !installed() {
        return;
    }
    // Without the call the defenders head for their sites.
    let spread = |hold: bool| {
        let (mut sim, me, bots) = dust2(0);
        let mut cursor = sim.app.world().resource::<Messages<Radio>>().get_cursor_current();
        if hold {
            call(&mut sim, me, "holdpos");
        }
        let heard = run(&mut sim, 20.0, &mut cursor);
        let spot = feet(&sim, me);
        let far = bots
            .iter()
            .map(|&b| (feet(&sim, b) - spot).xz().length())
            .fold(0.0, f32::max);
        let rogers = heard.iter().filter(|h| h.2 == "roger").count();
        (far, rogers)
    };
    let (held, rogers) = spread(true);
    assert!(held < HOLD_NEAR + 2.0, "a holding bot is {held:.1} m off the spot");
    assert!((1..=2).contains(&rogers), "one or two answer: {rogers}");
    let (free, rogers) = spread(false);
    assert!(free > 10.0, "without the call they leave ({free:.1} m)");
    assert_eq!(rogers, 0);
}

/// A radio with "Roger that" and CS:S-like join lines.
fn radio() -> RadioCommands {
    RadioCommands {
        commands: vec![RadioCommand {
            command: "roger".into(),
            menu: Some((2, 1)),
            variants: vec![("Radio.Roger".into(), "Roger that.".into())],
        }],
        format: "\u{2}%s1 (RADIO): %s2".into(),
        say: SayFormats {
            joins: vec![
                (1, "%s1 is joining the Terrorist force".into()),
                (2, "%s1 is joining the Counter-Terrorist force".into()),
            ],
            ..default()
        },
        ..default()
    }
}

/// Chat lines since the cursor last read, as plain text.
fn lines(sim: &Sim, cursor: &mut MessageCursor<ChatLine>) -> Vec<String> {
    let messages = sim.app.world().resource::<Messages<ChatLine>>();
    cursor
        .read(messages)
        .map(|l| l.0.iter().map(|r| r.1.as_str()).collect())
        .collect()
}

#[test]
fn ignorerad_hides_teammates_calls() {
    let mut sim = Sim::new(RadioHearPlugin);
    sim.app.insert_resource(radio());
    let me = sim.spawn_character(Vec3::new(0.0, 1.0, 0.0), placeholder::ID);
    sim.app.world_mut().entity_mut(me).insert((Team(2), LocalPlayer));
    let mate = sim.spawn_character(Vec3::new(3.0, 1.0, 0.0), placeholder::ID);
    sim.app
        .world_mut()
        .entity_mut(mate)
        .insert((Team(2), Name::new("Bot 1")));
    sim.ticks(1);
    let mut cursor = sim.app.world().resource::<Messages<ChatLine>>().get_cursor_current();
    let say = |sim: &mut Sim, who: Entity, cursor: &mut MessageCursor<ChatLine>| {
        call(sim, who, "roger");
        sim.ticks(1);
        lines(sim, cursor)
    };
    assert_eq!(say(&mut sim, mate, &mut cursor), ["Bot 1 (RADIO): Roger that."]);
    sim.app.world_mut().resource_mut::<Console>().submit("ignorerad 1");
    sim.ticks(1);
    assert!(say(&mut sim, mate, &mut cursor).is_empty(), "teammates are ignored");
    assert_eq!(
        say(&mut sim, me, &mut cursor),
        ["Player (RADIO): Roger that."],
        "your own calls still show"
    );
    sim.app.world_mut().resource_mut::<Console>().submit("ignorerad 0");
    sim.ticks(1);
    assert_eq!(say(&mut sim, mate, &mut cursor).len(), 1);
}

#[test]
fn joining_a_team_says_so_in_the_chat() {
    let mut sim = Sim::new(GameMessagesPlugin);
    let mut cursor = sim.app.world().resource::<Messages<ChatLine>>().get_cursor_current();
    let me = sim.spawn_character(Vec3::new(0.0, 1.0, 0.0), placeholder::ID);
    sim.app.world_mut().entity_mut(me).insert((Team(2), LocalPlayer));
    sim.ticks(1);
    assert!(
        lines(&sim, &mut cursor).is_empty(),
        "nothing before the game's strings load"
    );
    sim.app.insert_resource(radio());
    sim.ticks(1);
    assert_eq!(
        lines(&sim, &mut cursor),
        ["Player is joining the Counter-Terrorist force"]
    );
    let bot = sim.spawn_character(Vec3::new(3.0, 1.0, 0.0), placeholder::ID);
    sim.app
        .world_mut()
        .entity_mut(bot)
        .insert((Team(1), Name::new("Bot 1")));
    sim.ticks(2);
    assert_eq!(lines(&sim, &mut cursor), ["Bot 1 is joining the Terrorist force"]);
    sim.app.world_mut().entity_mut(me).insert(Team(1));
    sim.ticks(1);
    assert_eq!(lines(&sim, &mut cursor), ["Player is joining the Terrorist force"]);
    sim.ticks(3);
    assert!(lines(&sim, &mut cursor).is_empty(), "once per change");
}

#[test]
fn the_installs_join_lines() {
    if !installed() {
        return;
    }
    let map = games::load_map("cs_source:de_dust2").expect("load de_dust2");
    let say = &map.radio.as_ref().expect("CS:S has a radio").say;
    let line: String = say
        .join("Bot 2", 2)
        .expect("a CT join line")
        .iter()
        .map(|r| r.1.as_str())
        .collect();
    assert_eq!(line, "Bot 2 is joining the Counter-Terrorist force");
    assert!(say.join("Bot 3", 1).is_some());
}

/// de_dust2 with `t` terrorist and `ct` counter-terrorist bots, padded
/// by `pad` more entities; rounds on (`rounds`) with a 1 s freeze.
fn bot_game(t: usize, ct: usize, pad: usize, rounds: bool) -> (Sim, Vec<Entity>) {
    let map = games::load_map("cs_source:de_dust2").expect("load de_dust2");
    let mut sim = Sim::new((MapPlugin::new(map), SourceMovementPlugin, CsWeaponsPlugin));
    sim.pad(pad);
    sim.set_tick_interval(TICK_INTERVAL);
    sim.app
        .insert_resource(mashup::slots::Loadout { movement: movement::ID });
    sim.app.world_mut().resource_mut::<BotConfig>().grenades = 0;
    let mut bots = Vec::new();
    for _ in 0..t {
        bots.push(mashup::bot::add_bot(sim.app.world_mut(), Team(1)).expect("bot"));
    }
    for _ in 0..ct {
        bots.push(mashup::bot::add_bot(sim.app.world_mut(), Team(2)).expect("bot"));
    }
    if rounds {
        sim.app
            .world_mut()
            .resource_mut::<Console>()
            .submit("mp_freezetime 1; mp_roundtime 1.5; mashup_rounds 1");
    }
    sim.ticks(2);
    (sim, bots)
}

/// A call bots start on their own (not an answer, nor a grenade's "Fire
/// in the hole", which the weapon says).
fn own(call: &str) -> bool {
    CALLS.contains(&call)
}

fn live(sim: &Sim) -> bool {
    matches!(sim.app.world().resource::<RoundState>().phase, Phase::Live { .. })
}

#[test]
fn a_round_opens_with_one_leader_call() {
    if !installed() {
        return;
    }
    let (mut sim, bots) = bot_game(4, 1, 0, true);
    let mut cursor = sim.app.world().resource::<Messages<Radio>>().get_cursor_current();
    // The freeze: nobody says anything of their own.
    let mut frozen = Vec::new();
    while !live(&sim) {
        frozen.extend(run(&mut sim, TICK_INTERVAL, &mut cursor));
    }
    assert!(frozen.iter().all(|h| !own(&h.2)), "said in the freeze: {frozen:?}");
    let heard = run(&mut sim, 9.0, &mut cursor);
    let leader = {
        let t = sim.app.world().resource::<Tactics>();
        t.team(Team(1)).and_then(|p| p.leader).expect("a leader")
    };
    let starts: Vec<&(u64, Entity, String)> = heard
        .iter()
        .filter(|h| bots[..4].contains(&h.1) && matches!(h.2.as_str(), "go" | "sticktog" | "followme"))
        .collect();
    assert_eq!(starts.len(), 1, "one call to start the round: {heard:?}");
    assert_eq!(starts[0].1, leader, "the leader makes it");
    // Teammates near answer it.
    assert!(
        heard.iter().any(|h| h.2 == "roger" && bots[..4].contains(&h.1)),
        "answered: {heard:?}"
    );
}

#[test]
fn a_planting_bot_calls_cover_me() {
    if !installed() {
        return;
    }
    let (mut sim, bots) = bot_game(2, 0, 0, false);
    let (planter, mate) = (bots[0], bots[1]);
    // Both in the A target (O4/O5's box), the planter with the bomb.
    let teleport = |sim: &mut Sim, e: Entity, x: f32, y: f32| {
        let at = to_engine(Vec3::new(x, y, 100.0)) + Vec3::Y * (36.0 * mashup::objectives::UNIT + 0.05);
        let w = sim.app.world_mut();
        w.get_mut::<Transform>(e).unwrap().translation = at;
        if let Some(mut p) = w.get_mut::<Position>(e) {
            p.0 = at;
        }
    };
    teleport(&mut sim, planter, 1160.0, 2480.0);
    teleport(&mut sim, mate, 1120.0, 2400.0);
    give(sim.app.world_mut(), planter, C4).expect("the bomb");
    let mut cursor = sim.app.world().resource::<Messages<Radio>>().get_cursor_current();
    let heard = run(&mut sim, 4.0, &mut cursor);
    assert!(
        heard.iter().any(|h| h.1 == planter && h.2 == "coverme"),
        "the planter asks for cover: {heard:?}"
    );
    assert!(
        heard.iter().any(|h| h.1 == mate && h.2 == "roger"),
        "the mate answers: {heard:?}"
    );
    let bot = sim.app.world().get::<Bot>(planter).unwrap();
    assert!(bot.last_call().is_some(), "remembered for the debug views");
}

/// A terrorists-only round's own calls: (tick from the round's start,
/// bot number, call).
fn attackers_calls(pad: usize) -> Vec<(u64, u32, String)> {
    let (mut sim, _) = bot_game(4, 0, pad, false);
    let mut cursor = sim.app.world().resource::<Messages<Radio>>().get_cursor_current();
    let start = sim.tick();
    let heard = run(&mut sim, 30.0, &mut cursor);
    heard
        .into_iter()
        .map(|h| (h.0 - start, number(&sim, h.1), h.2))
        .collect()
}

#[test]
fn bots_calls_dont_depend_on_entity_ids() {
    if !installed() {
        return;
    }
    let base = attackers_calls(0);
    assert!(base.iter().any(|c| own(&c.2)), "something said: {base:?}");
    for pad in [7, 40] {
        assert_eq!(attackers_calls(pad), base, "padded by {pad}");
    }
}

#[test]
fn a_teams_bots_start_a_call_per_gap_at_most_over_a_round() {
    if !installed() {
        return;
    }
    let (mut sim, bots) = bot_game(4, 4, 0, true);
    let mut cursor = sim.app.world().resource::<Messages<Radio>>().get_cursor_current();
    let mut heard = Vec::new();
    let mut was_live = false;
    // Until the first round is over (at most its 90 s and the freeze).
    for _ in 0..(100.0 / 0.5) as usize {
        heard.extend(run(&mut sim, 0.5, &mut cursor));
        let now_live = live(&sim);
        if was_live && !now_live {
            break;
        }
        was_live |= now_live;
    }
    assert!(was_live, "the round went live");
    let tick_gap = (TEAM_GAP / TICK_INTERVAL).round() as u64 - 1;
    for team in [1u8, 2] {
        let calls: Vec<&(u64, Entity, String)> = heard
            .iter()
            .filter(|h| bots.contains(&h.1) && own(&h.2))
            .filter(|h| sim.app.world().get::<Team>(h.1) == Some(&Team(team)))
            .collect();
        eprintln!(
            "team {team}: {:?}",
            calls.iter().map(|c| (c.0, &c.2)).collect::<Vec<_>>()
        );
        for w in calls.windows(2) {
            assert!(
                w[1].0 - w[0].0 >= tick_gap,
                "team {team}: {:?} then {:?} too soon",
                w[0],
                w[1]
            );
        }
    }
    assert!(heard.iter().any(|h| own(&h.2)), "bots said something");
}
