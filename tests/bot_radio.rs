//! Bots answering teammates' radio commands (`bot::radio::obey`): a
//! person's "Follow me" makes the nearest bot answer and follow them,
//! "Hold this position" keeps bots near the caller's spot, all the same
//! whatever the entity ids (`MASHUP_TEST_PAD`); on de_dust2, skipped
//! without a CS:S install. And what the player hears: `ignorerad` hides
//! teammates' calls, and players joining a team get the game's chat line.

use bevy::{
    ecs::message::{MessageCursor, Messages},
    prelude::*,
};
use mashup::{
    bot::{
        Activity, Bot, BotConfig,
        radio::{HOLD_NEAR, Order},
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
            movement::{self, SourceMovementPlugin},
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
