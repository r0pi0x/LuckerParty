// mashup_probe: replays per-tick inputs on a CS:S bot and logs the result,
// so mashup's Source movement can be compared with the real game tick for
// tick (docs/OBSERVABILITY.md, "Tick-exact CS:S comparisons").
//
// Server command:  mashup_run <input file> <output file>   (paths relative
// to the game folder). Runs on the first living bot; use bot_zombie 1 so
// the bot's own AI doesn't fight the inputs.
//
// Input file: one optional "start x y z pitch yaw vx vy vz" line, then one
// line per tick: "buttons forwardmove sidemove pitch yaw".
// Output file: one line per tick, "tick x y z vx vy vz onground ducked
// hull_top eye buttons water ladder" (the collision box's top, the eye
// height above the origin, the buttons the movement actually ran with, the
// water level 0-3 and 1 when on a ladder);
// tick 0 is the state after the bot was placed at the start (one idle
// tick), tick n the state after input line n.
//
// Weapon measurements (tools/css_probe/weapcmp.py): `mashup_wrun` is
// mashup_run that keeps the bot's current weapon and adds, after each tick row, a
// "W tick ..." row with the weapon state and "E tick <event> ..." rows for
// weapon_fire, bullet_impact, player_hurt, weapon_reload, weapon_zoom and
// player_death (tick = the input line whose command raised the event).
// The shooter is the run's bot (first living bot); the target is the first
// other living player. mashup_give, mashup_target and mashup_trace set up
// and aim the shots.

#include <sourcemod>
#include <sdktools>
#include <cstrike>

public Plugin myinfo = {
    name = "mashup probe",
    author = "mashup",
    description = "Replays per-tick inputs on a bot and logs its movement",
    version = "1",
};

#define MAX_TICKS 16384

int g_buttons[MAX_TICKS];
float g_forward[MAX_TICKS];
float g_side[MAX_TICKS];
float g_pitch[MAX_TICKS];
float g_yaw[MAX_TICKS];
// Optional sixth input column: 1 + the weapon slot to select this tick.
int g_select[MAX_TICKS];
int g_count;

float g_startPos[3];
float g_startAng[3];
float g_startVel[3];
bool g_hasStart;

int g_bot;
File g_out;
// -1: place the bot on its next command; then the index of the next input.
int g_cursor = -1;
// Weapon the bot holds for runs (its max speed matters).
char g_weapon[64] = "weapon_knife";

// Weapon logging (see the header).
bool g_wlog;
int g_seed;
int g_cmdnum;
// Target state restored after every hit, so each shot meets the same target.
int g_tHealth = 1000;
// The target's view yaw, forced every command (the bot AI turns it otherwise).
float g_tYaw;
int g_target;
// Entity the trace commands skip (-1: none).
int g_traceIgnore = -1;
int g_tArmor;
bool g_tHelmet;

public void OnPluginStart()
{
    RegServerCmd("mashup_run", Command_Run, "mashup_run <input> <output>");
    RegServerCmd("mashup_weapon", Command_Weapon, "mashup_weapon <weapon_name>: what the bot holds for runs");
    RegServerCmd("mashup_wrun", Command_WRun, "mashup_wrun <input> <output>: mashup_run with weapon logging");
    RegServerCmd("mashup_give", Command_Give,
        "mashup_give <weapon> [clip] [reserve]: replace the shooter's weapon in that slot and draw it");
    RegServerCmd("mashup_target", Command_Target,
        "mashup_target <x> <y> <z> <yaw> <health> <armor> <helmet 0|1> <hover 0|1>: place the target");
    RegServerCmd("mashup_trace", Command_Trace,
        "mashup_trace <x> <y> <z> <pitch> <yaw> [ignore shooter 0|1]: bullet-mask trace, prints the hit");
    RegServerCmd("mashup_scan", Command_Scan,
        "mashup_scan <x0> <x1> <y0> <y1> <step> <ztop> <mindist>: ground points with long level sightlines");
    RegServerCmd("mashup_info", Command_Info, "mashup_info: position and angles of all living players");
    HookEvent("weapon_fire", Event_Any);
    HookEvent("bullet_impact", Event_Any);
    HookEvent("player_hurt", Event_Any);
    HookEvent("weapon_reload", Event_Any);
    HookEvent("weapon_zoom", Event_Any);
    HookEvent("player_death", Event_Any);
}

int FindTarget2()
{
    int shooter = FindBot();
    for (int i = 1; i <= MaxClients; i++) {
        if (i != shooter && IsClientInGame(i) && GetClientTeam(i) > 1) {
            // A bot that joined mid-round waits dead for the next round,
            // which never comes with mp_ignore_round_win_conditions.
            if (!IsPlayerAlive(i)) {
                CS_RespawnPlayer(i);
            }
            return i;
        }
    }
    return 0;
}

public Action Command_WRun(int args)
{
    return StartRun(true);
}

public Action Command_Give(int args)
{
    char name[64];
    GetCmdArg(1, name, sizeof(name));
    int bot = FindBot();
    if (bot == 0) {
        PrintToServer("mashup: no living bot");
        return Plugin_Handled;
    }
    // Remove guns (slots 0 and 1) and any weapon of the same class first,
    // so the bot holds only this one and its knife.
    for (int slot = 0; slot < 5; slot++) {
        int w = GetPlayerWeaponSlot(bot, slot);
        if (w == -1) {
            continue;
        }
        char cls[64];
        GetEntityClassname(w, cls, sizeof(cls));
        if (StrEqual(cls, name) || slot <= 1) {
            RemovePlayerItem(bot, w);
            AcceptEntityInput(w, "Kill");
        }
    }
    int w = GivePlayerItem(bot, name);
    if (w == -1) {
        PrintToServer("mashup: cannot give %s", name);
        return Plugin_Handled;
    }
    if (args >= 2) {
        SetEntProp(w, Prop_Send, "m_iClip1", GetCmdArgInt(2));
    }
    if (args >= 3) {
        int type = GetEntProp(w, Prop_Send, "m_iPrimaryAmmoType");
        if (type >= 0) {
            SetEntProp(bot, Prop_Send, "m_iAmmo", GetCmdArgInt(3), _, type);
        }
    }
    CreateTimer(0.1, Timer_Use, bot);
    PrintToServer("mashup: gave %s (entity %d) to %d", name, w, bot);
    return Plugin_Handled;
}

public Action Timer_Use(Handle timer, int bot)
{
    if (IsClientInGame(bot) && IsPlayerAlive(bot)) {
        int w = GetPlayerWeaponSlot(bot, 2);
        int best = -1;
        for (int slot = 0; slot < 2 && best == -1; slot++) {
            best = GetPlayerWeaponSlot(bot, slot);
        }
        if (best == -1) {
            best = w;
        }
        if (best != -1) {
            char cls[64];
            GetEntityClassname(best, cls, sizeof(cls));
            FakeClientCommand(bot, "use %s", cls);
        }
    }
    return Plugin_Stop;
}

public void OnGameFrame()
{
    if (g_target != 0 && IsClientInGame(g_target) && IsPlayerAlive(g_target)) {
        float a[3];
        a[1] = g_tYaw;
        SetEntPropVector(g_target, Prop_Data, "v_angle", a);
        TeleportEntity(g_target, NULL_VECTOR, a, NULL_VECTOR);
    }
}

public Action Command_Scan(int args)
{
    float x0 = GetCmdArgFloat(1);
    float x1 = GetCmdArgFloat(2);
    float y0 = GetCmdArgFloat(3);
    float y1 = GetCmdArgFloat(4);
    float step = GetCmdArgFloat(5);
    float ztop = GetCmdArgFloat(6);
    float minDist = GetCmdArgFloat(7);
    g_traceIgnore = -1;
    for (float x = x0; x <= x1; x += step) {
        for (float y = y0; y <= y1; y += step) {
            float top[3];
            float down[3] = {90.0, 0.0, 0.0};
            top[0] = x;
            top[1] = y;
            top[2] = ztop;
            TR_TraceRayFilter(top, down, MASK_PLAYERSOLID, RayType_Infinite, TraceFilter);
            if (TR_StartSolid() || TR_GetFraction() >= 1.0) {
                continue;
            }
            float ground[3];
            TR_GetEndPosition(ground);
            for (int k = 0; k < 24; k++) {
                float ang[3];
                ang[1] = 15.0 * k;
                // Shortest clear distance at heights 4, 36 and 64 above the
                // ground: the whole body has to be visible.
                float d = 1.0e9;
                for (int h = 0; h < 3; h++) {
                    float eye[3];
                    eye = ground;
                    eye[2] += h == 0 ? 4.0 : (h == 1 ? 36.0 : 64.0);
                    TR_TraceRayFilter(eye, ang, MASK_SHOT, RayType_Infinite, TraceFilter);
                    float end[3];
                    TR_GetEndPosition(end);
                    float dh = GetVectorDistance(eye, end);
                    if (dh < d) {
                        d = dh;
                    }
                }
                if (d >= minDist) {
                    PrintToServer("mashup_scan %.0f %.0f %.1f yaw %.0f dist %.0f", x, y, ground[2], ang[1], d);
                }
            }
        }
    }
    return Plugin_Handled;
}

public Action Command_Info(int args)
{
    for (int i = 1; i <= MaxClients; i++) {
        if (!IsClientInGame(i) || !IsPlayerAlive(i)) {
            continue;
        }
        float o[3];
        float abs[3];
        float eye[3];
        float v[3];
        GetClientAbsOrigin(i, o);
        GetClientAbsAngles(i, abs);
        GetClientEyeAngles(i, eye);
        GetEntPropVector(i, Prop_Data, "v_angle", v);
        PrintToServer("mashup_info %d team %d origin %.3f %.3f %.3f abs %.3f %.3f eye %.3f %.3f v_angle %.3f %.3f health %d armor %d",
            i, GetClientTeam(i), o[0], o[1], o[2], abs[0], abs[1], eye[0], eye[1], v[0], v[1], GetClientHealth(i),
            GetEntProp(i, Prop_Send, "m_ArmorValue"));
    }
    return Plugin_Handled;
}

void ResetTarget(int t)
{
    SetEntityHealth(t, g_tHealth);
    SetEntProp(t, Prop_Send, "m_ArmorValue", g_tArmor);
    SetEntProp(t, Prop_Send, "m_bHasHelmet", g_tHelmet ? 1 : 0);
}

public Action Command_Target(int args)
{
    int t = FindTarget2();
    if (t == 0) {
        PrintToServer("mashup: no target");
        return Plugin_Handled;
    }
    float pos[3];
    float ang[3];
    float zero[3];
    pos[0] = GetCmdArgFloat(1);
    pos[1] = GetCmdArgFloat(2);
    pos[2] = GetCmdArgFloat(3);
    ang[1] = GetCmdArgFloat(4);
    g_tYaw = ang[1];
    g_target = t;
    g_tHealth = GetCmdArgInt(5);
    g_tArmor = GetCmdArgInt(6);
    g_tHelmet = GetCmdArgInt(7) != 0;
    SetEntityMoveType(t, GetCmdArgInt(8) != 0 ? MOVETYPE_NONE : MOVETYPE_WALK);
    TeleportEntity(t, pos, ang, zero);
    ResetTarget(t);
    PrintToServer("mashup: target %d at %.1f %.1f %.1f yaw %.1f", t, pos[0], pos[1], pos[2], ang[1]);
    return Plugin_Handled;
}

public bool TraceFilter(int entity, int mask)
{
    return entity != g_traceIgnore;
}

public Action Command_Trace(int args)
{
    float pos[3];
    float ang[3];
    pos[0] = GetCmdArgFloat(1);
    pos[1] = GetCmdArgFloat(2);
    pos[2] = GetCmdArgFloat(3);
    ang[0] = GetCmdArgFloat(4);
    ang[1] = GetCmdArgFloat(5);
    g_traceIgnore = (args >= 6 && GetCmdArgInt(6) != 0) ? FindBot() : -1;
    TR_TraceRayFilter(pos, ang, MASK_SHOT, RayType_Infinite, TraceFilter);
    float end[3];
    float n[3];
    TR_GetEndPosition(end);
    TR_GetPlaneNormal(null, n);
    char surf[64];
    TR_GetSurfaceName(null, surf, sizeof(surf));
    PrintToServer("mashup_trace %.3f %.3f %.3f ent %d group %d frac %.6f startsolid %d normal %.3f %.3f %.3f props %d surf %s",
        end[0], end[1], end[2], TR_GetEntityIndex(), TR_GetHitGroup(), TR_GetFraction(), TR_StartSolid() ? 1 : 0,
        n[0], n[1], n[2], TR_GetSurfaceProps(), surf);
    return Plugin_Handled;
}

public void Event_Any(Event event, const char[] name, bool dontBroadcast)
{
    if (!g_wlog || g_out == null) {
        return;
    }
    int tick = g_cursor + 1;
    int who = GetClientOfUserId(event.GetInt("userid"));
    if (StrEqual(name, "bullet_impact")) {
        g_out.WriteLine("E %d bullet_impact %d %.4f %.4f %.4f", tick, who, event.GetFloat("x"), event.GetFloat("y"),
            event.GetFloat("z"));
    } else if (StrEqual(name, "player_hurt")) {
        char weapon[64];
        event.GetString("weapon", weapon, sizeof(weapon));
        float abs[3];
        float eye[3];
        if (who != 0) {
            GetClientAbsAngles(who, abs);
            GetClientEyeAngles(who, eye);
        }
        g_out.WriteLine("E %d player_hurt %d attacker %d health %d armor %d dmg_health %d dmg_armor %d hitgroup %d absyaw %.3f eyeyaw %.3f weapon %s",
            tick, who, GetClientOfUserId(event.GetInt("attacker")), event.GetInt("health"), event.GetInt("armor"),
            event.GetInt("dmg_health"), event.GetInt("dmg_armor"), event.GetInt("hitgroup"), abs[1], eye[1], weapon);
        if (who != 0 && who != g_bot && IsPlayerAlive(who)) {
            ResetTarget(who);
        }
    } else {
        char weapon[64];
        event.GetString("weapon", weapon, sizeof(weapon));
        g_out.WriteLine("E %d %s %d %s", tick, name, who, weapon);
    }
}

void WriteWeaponRow(int client)
{
    int w = GetEntPropEnt(client, Prop_Send, "m_hActiveWeapon");
    char cls[64] = "none";
    int clip = -1;
    int reserve = -1;
    float np = 0.0;
    float ns = 0.0;
    float pen = 0.0;
    int mode = 0;
    int reload = 0;
    int special = 0;
    if (w != -1) {
        GetEntityClassname(w, cls, sizeof(cls));
        clip = GetEntProp(w, Prop_Send, "m_iClip1");
        int type = GetEntProp(w, Prop_Send, "m_iPrimaryAmmoType");
        if (type >= 0) {
            reserve = GetEntProp(client, Prop_Send, "m_iAmmo", _, type);
        }
        np = GetEntPropFloat(w, Prop_Send, "m_flNextPrimaryAttack");
        ns = GetEntPropFloat(w, Prop_Send, "m_flNextSecondaryAttack");
        if (HasEntProp(w, Prop_Send, "m_fAccuracyPenalty")) {
            pen = GetEntPropFloat(w, Prop_Send, "m_fAccuracyPenalty");
        }
        if (HasEntProp(w, Prop_Send, "m_weaponMode")) {
            mode = GetEntProp(w, Prop_Send, "m_weaponMode");
        }
        reload = GetEntProp(w, Prop_Data, "m_bInReload");
        if (HasEntProp(w, Prop_Send, "m_bSilencerOn")) {
            special = GetEntProp(w, Prop_Send, "m_bSilencerOn");
        } else if (HasEntProp(w, Prop_Send, "m_bBurstMode")) {
            special = GetEntProp(w, Prop_Send, "m_bBurstMode");
        }
    }
    float punch[3];
    float punchVel[3];
    float eye[3];
    float eyeAng[3];
    GetEntPropVector(client, Prop_Data, "m_vecPunchAngle", punch);
    GetEntPropVector(client, Prop_Data, "m_vecPunchAngleVel", punchVel);
    GetClientEyePosition(client, eye);
    // The command's view angles (what shots use); GetClientEyeAngles gives
    // the bot's networked look angles instead.
    GetEntPropVector(client, Prop_Data, "v_angle", eyeAng);
    g_out.WriteLine("W %d %s clip %d reserve %d tickbase %d next_primary %.5f next_secondary %.5f next_attack %.5f shots %d punch %.5f %.5f %.5f punchvel %.5f %.5f %.5f fov %d penalty %.6f mode %d special %d reload %d eye %.4f %.4f %.4f ang %.4f %.4f seed %d cmd %d velmod %.4f",
        g_cursor, cls, clip, reserve, GetEntProp(client, Prop_Send, "m_nTickBase"), np, ns,
        GetEntPropFloat(client, Prop_Send, "m_flNextAttack"), GetEntProp(client, Prop_Send, "m_iShotsFired"),
        punch[0], punch[1], punch[2], punchVel[0], punchVel[1], punchVel[2], GetEntProp(client, Prop_Send, "m_iFOV"),
        pen, mode, special, reload, eye[0], eye[1], eye[2], eyeAng[0], eyeAng[1], g_seed, g_cmdnum,
        GetEntPropFloat(client, Prop_Send, "m_flVelocityModifier"));
}

int FindBot()
{
    for (int i = 1; i <= MaxClients; i++) {
        if (IsClientInGame(i) && IsFakeClient(i) && IsPlayerAlive(i)) {
            return i;
        }
    }
    return 0;
}

public Action Command_Weapon(int args)
{
    GetCmdArg(1, g_weapon, sizeof(g_weapon));
    int bot = FindBot();
    if (bot != 0 && GetPlayerWeaponSlot(bot, 0) == -1 && !StrEqual(g_weapon, "weapon_knife")) {
        GivePlayerItem(bot, g_weapon);
    }
    PrintToServer("mashup: bot weapon %s", g_weapon);
    return Plugin_Handled;
}

public Action Command_Run(int args)
{
    // Plain movement runs: no weapon rows, no target held in place.
    g_target = 0;
    return StartRun(false);
}

Action StartRun(bool wlog)
{
    g_wlog = wlog;
    char inPath[PLATFORM_MAX_PATH];
    char outPath[PLATFORM_MAX_PATH];
    GetCmdArg(1, inPath, sizeof(inPath));
    GetCmdArg(2, outPath, sizeof(outPath));

    File f = OpenFile(inPath, "r");
    if (f == null) {
        PrintToServer("mashup: cannot open %s", inPath);
        return Plugin_Handled;
    }
    g_count = 0;
    g_hasStart = false;
    char line[256];
    char parts[10][32];
    while (f.ReadLine(line, sizeof(line))) {
        TrimString(line);
        if (line[0] == '\0') {
            continue;
        }
        int n = ExplodeString(line, " ", parts, sizeof(parts), sizeof(parts[]));
        if (StrEqual(parts[0], "start") && n >= 9) {
            for (int k = 0; k < 3; k++) {
                g_startPos[k] = StringToFloat(parts[1 + k]);
                g_startVel[k] = StringToFloat(parts[6 + k]);
            }
            g_startAng[0] = StringToFloat(parts[4]);
            g_startAng[1] = StringToFloat(parts[5]);
            g_startAng[2] = 0.0;
            g_hasStart = true;
        } else if (n >= 5 && g_count < MAX_TICKS) {
            g_buttons[g_count] = StringToInt(parts[0]);
            g_forward[g_count] = StringToFloat(parts[1]);
            g_side[g_count] = StringToFloat(parts[2]);
            g_pitch[g_count] = StringToFloat(parts[3]);
            g_yaw[g_count] = StringToFloat(parts[4]);
            g_select[g_count] = n >= 6 ? StringToInt(parts[5]) : 0;
            g_count++;
        }
    }
    delete f;

    g_bot = FindBot();
    if (g_bot == 0) {
        PrintToServer("mashup: no living bot");
        return Plugin_Handled;
    }
    if (g_out != null) {
        delete g_out;
    }
    g_out = OpenFile(outPath, "w");
    if (g_out == null) {
        PrintToServer("mashup: cannot write %s", outPath);
        return Plugin_Handled;
    }
    // Knife by default: max speed 250, the value mashup assumes. Weapon
    // runs keep whatever the bot holds.
    if (!g_wlog) {
        FakeClientCommand(g_bot, "use %s", g_weapon);
    }
    g_cursor = -1;
    PrintToServer("mashup: running %d ticks on client %d", g_count, g_bot);
    return Plugin_Handled;
}

public Action OnPlayerRunCmd(int client, int &buttons, int &impulse, float vel[3], float angles[3],
    int &weapon, int &subtype, int &cmdnum, int &tickcount, int &seed, int mouse[2])
{
    if (client == g_target && client != g_bot && g_target != 0) {
        angles[0] = 0.0;
        angles[1] = g_tYaw;
        angles[2] = 0.0;
        SetEntPropVector(client, Prop_Data, "v_angle", angles);
        TeleportEntity(client, NULL_VECTOR, angles, NULL_VECTOR);
        return Plugin_Changed;
    }
    if (client != g_bot || g_out == null) {
        return Plugin_Continue;
    }
    g_seed = seed;
    g_cmdnum = cmdnum;
    if (g_cursor < 0) {
        if (g_hasStart) {
            // Off any ladder from a previous run, and forget it: CS:S keeps
            // the last ladder's normal and probes along it without input.
            SetEntityMoveType(client, MOVETYPE_WALK);
            if (HasEntProp(client, Prop_Data, "m_vecLadderNormal")) {
                float zero[3];
                SetEntPropVector(client, Prop_Data, "m_vecLadderNormal", zero);
            }
            // No jump stamina left over from the previous run.
            if (HasEntProp(client, Prop_Send, "m_flStamina")) {
                SetEntPropFloat(client, Prop_Send, "m_flStamina", 0.0);
            }
            TeleportEntity(client, g_startPos, g_startAng, g_startVel);
            angles[0] = g_startAng[0];
            angles[1] = g_startAng[1];
        }
        buttons = 0;
        vel[0] = 0.0;
        vel[1] = 0.0;
        vel[2] = 0.0;
        return Plugin_Changed;
    }
    if (g_cursor >= g_count) {
        return Plugin_Continue;
    }
    buttons = g_buttons[g_cursor];
    vel[0] = g_forward[g_cursor];
    vel[1] = g_side[g_cursor];
    vel[2] = 0.0;
    angles[0] = g_pitch[g_cursor];
    angles[1] = g_yaw[g_cursor];
    angles[2] = 0.0;
    if (g_select[g_cursor] > 0) {
        int w = GetPlayerWeaponSlot(client, g_select[g_cursor] - 1);
        if (w != -1) {
            weapon = w;
        }
    } else if (g_wlog) {
        // No weapon changes from the bot AI during weapon runs.
        weapon = 0;
    }
    if (g_wlog) {
        // Shots use the player's view angles, which the bot AI sets on its
        // own; force them to the command's.
        SetEntPropVector(client, Prop_Data, "v_angle", angles);
        TeleportEntity(client, NULL_VECTOR, angles, NULL_VECTOR);
    }
    return Plugin_Changed;
}

public void OnPlayerRunCmdPost(int client, int buttons, int impulse, const float vel[3], const float angles[3],
    int weapon, int subtype, int cmdnum, int tickcount, int seed, const int mouse[2])
{
    if (client != g_bot || g_out == null) {
        return;
    }
    if (g_cursor >= g_count) {
        delete g_out;
        g_out = null;
        PrintToServer("mashup: done");
        return;
    }
    float o[3];
    float v[3];
    GetClientAbsOrigin(client, o);
    GetEntPropVector(client, Prop_Data, "m_vecAbsVelocity", v);
    float maxs[3];
    float view[3];
    GetEntPropVector(client, Prop_Send, "m_vecMaxs", maxs);
    GetEntPropVector(client, Prop_Data, "m_vecViewOffset", view);
    int flags = GetEntityFlags(client);
    int water = GetEntProp(client, Prop_Data, "m_nWaterLevel");
    int ladder = (GetEntityMoveType(client) == MOVETYPE_LADDER) ? 1 : 0;
    g_cursor++;
    g_out.WriteLine("%d %.4f %.4f %.4f %.4f %.4f %.4f %d %d %.4f %.4f %d %d %d", g_cursor, o[0], o[1], o[2], v[0], v[1],
        v[2], (flags & FL_ONGROUND) ? 1 : 0, (flags & FL_DUCKING) ? 1 : 0, maxs[2], view[2], buttons, water, ladder);
    if (g_wlog) {
        WriteWeaponRow(client);
    }
}
